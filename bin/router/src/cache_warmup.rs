//! Plans known operations against a new supergraph before the router switches to it.
//!
//! Only the plan cache gets warmed. Its key comes from the normalized operation itself,
//! so a warmed plan is right no matter how a request later arrives at that operation.
//! The parse, validation and normalization caches are keyed by what the client sent and
//! what plugins did to it, which warmup can't reproduce, so those stay cold. They're cheap.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{debug, info};

use crate::config::HiveRouterConfig;
use crate::executor::hooks::on_cache_warmup::{CacheWarmupOperation, OnCacheWarmupHookPayload};
use crate::executor::hooks::on_supergraph_load::SupergraphSnapshot;
use crate::executor::plugin_trait::RouterPluginBoxed;
use crate::pipeline::normalize::normalize_document;
use crate::pipeline::progressive_override::{RequestOverrideContext, StableOverrideContext};
use crate::pipeline::query_plan::{calculate_cache_key, plan_normalized_operation};
use crate::query_planner::state::supergraph_state::OperationKind;
use crate::query_planner::utils::cancellation::CancellationToken;
use crate::query_planner::utils::parsing::{
    safe_parse_operation, safe_parse_operation_with_token_limit,
};
use crate::schema_state::RouterSupergraphRuntime;
use crate::telemetry::logging::targets;
use crate::telemetry::metrics::cache_warmup_metrics::CacheWarmupMetrics;
use crate::telemetry::metrics::catalog::values::{
    CacheWarmupOperationResult, CacheWarmupResult, CacheWarmupSource,
};

pub(crate) struct CacheWarmup {
    timeout: Duration,
    max_tokens: Option<usize>,
    plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
}

impl CacheWarmup {
    /// `None` when warmup is turned off.
    pub fn from_config(
        router_config: &HiveRouterConfig,
        plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
    ) -> Option<Self> {
        let config = &router_config.cache.warmup;
        config.enabled.then(|| Self {
            timeout: config.timeout,
            max_tokens: router_config.limits.max_tokens.as_ref().map(|cfg| cfg.n),
            plugins,
        })
    }

    /// Fills `runtime`'s plan cache. Returns once everything is planned or the timeout runs out.
    pub async fn run(
        &self,
        previous: Option<&RouterSupergraphRuntime>,
        supergraph: &SupergraphSnapshot,
        runtime: &Arc<RouterSupergraphRuntime>,
        metrics: &CacheWarmupMetrics,
    ) {
        let started_at = Instant::now();
        let deadline = started_at + self.timeout;

        let mut operations = Vec::new();

        if let Some(previous) = previous {
            for (_, payload) in previous.normalize_cache.iter() {
                operations.push(CacheWarmupOperation::from_text(
                    payload.source_query.clone(),
                    payload.requested_operation_name.clone(),
                    CacheWarmupSource::PreviousSupergraph,
                ));
            }
        }

        if let Some(plugins) = &self.plugins {
            let mut payload = OnCacheWarmupHookPayload {
                supergraph,
                operations,
            };
            for plugin in plugins.iter() {
                plugin.on_cache_warmup(&mut payload).await;
            }
            operations = payload.operations;
        }

        let mut counts = Counts::default();
        let mut remaining = operations.into_iter();
        for operation in remaining.by_ref() {
            if Instant::now() >= deadline {
                counts.record(
                    metrics,
                    operation.source,
                    CacheWarmupOperationResult::TimedOut,
                );
                break;
            }
            let source = operation.source;
            let result = self
                .warm_one(operation, supergraph, runtime, deadline)
                .await;
            counts.record(metrics, source, result);
        }
        for operation in remaining {
            counts.record(
                metrics,
                operation.source,
                CacheWarmupOperationResult::TimedOut,
            );
        }

        let elapsed = started_at.elapsed();
        let run_result = if counts.timed_out > 0 {
            CacheWarmupResult::TimedOut
        } else {
            CacheWarmupResult::Completed
        };
        metrics.record_run(run_result, elapsed);
        info!(
            target: targets::SUPERGRAPH,
            warmed = counts.warmed,
            failed = counts.failed,
            skipped = counts.skipped,
            timed_out = counts.timed_out,
            elapsed_ms = elapsed.as_millis(),
            "plan cache warmup finished"
        );
    }

    async fn warm_one(
        &self,
        operation: CacheWarmupOperation,
        supergraph: &SupergraphSnapshot,
        runtime: &Arc<RouterSupergraphRuntime>,
        deadline: Instant,
    ) -> CacheWarmupOperationResult {
        let supergraph = supergraph.clone();
        let planning_runtime = runtime.clone();
        let max_tokens = self.max_tokens;
        let operation_name = operation.operation_name.clone();

        // parsing, normalizing and planning are CPU work, keep it off the async thread
        let planned = ntex::rt::spawn_blocking(move || {
            let document = match max_tokens {
                Some(limit) => safe_parse_operation_with_token_limit(&operation.query, limit),
                None => safe_parse_operation(&operation.query),
            }
            .map_err(|error| error.to_string())?;
            let normalized = normalize_document(
                &supergraph,
                &document,
                operation.query.clone(),
                operation.operation_name.as_deref(),
                OperationKind::Query,
                String::new(),
            )
            .map_err(|error| error.to_string())?;
            let override_context = default_override_context();
            let key = calculate_cache_key(
                normalized.operation_for_plan_hash,
                &StableOverrideContext::new(&supergraph.planner.supergraph, &override_context),
            );
            if planning_runtime.plan_cache.contains_key(&key) {
                return Ok(None);
            }
            let cancellation_token =
                CancellationToken::with_timeout(deadline.saturating_duration_since(Instant::now()));
            plan_normalized_operation(
                &supergraph,
                &planning_runtime,
                &normalized.operation_for_plan,
                &normalized,
                &override_context,
                &cancellation_token,
            )
            .map(|planned| Some((key, planned)))
            .map_err(|error| error.to_string())
        })
        .await;

        match planned {
            Ok(Ok(Some((key, planned)))) => {
                runtime.plan_cache.insert(key, planned).await;
                CacheWarmupOperationResult::Warmed
            }
            Ok(Ok(None)) => CacheWarmupOperationResult::Skipped,
            Ok(Err(_)) if Instant::now() >= deadline => CacheWarmupOperationResult::TimedOut,
            Ok(Err(error)) => {
                debug!(
                    target: targets::SUPERGRAPH,
                    operation_name = ?operation_name,
                    %error,
                    "failed to warm up the plan of an operation"
                );
                CacheWarmupOperationResult::Failed
            }
            Err(error) => {
                debug!(target: targets::SUPERGRAPH, %error, "cache warmup task did not finish");
                CacheWarmupOperationResult::Failed
            }
        }
    }
}

/// What a request with no override labels gets.
/// Plans for requests with active labels or percentages get built on their first request.
fn default_override_context() -> RequestOverrideContext {
    RequestOverrideContext {
        active_flags: HashSet::new(),
        // above every threshold, so no percentage-based override applies
        percentage_value: u64::MAX,
    }
}

#[derive(Default)]
struct Counts {
    warmed: u64,
    failed: u64,
    skipped: u64,
    timed_out: u64,
}

impl Counts {
    fn record(
        &mut self,
        metrics: &CacheWarmupMetrics,
        source: CacheWarmupSource,
        result: CacheWarmupOperationResult,
    ) {
        match result {
            CacheWarmupOperationResult::Warmed => self.warmed += 1,
            CacheWarmupOperationResult::Failed => self.failed += 1,
            CacheWarmupOperationResult::Skipped => self.skipped += 1,
            CacheWarmupOperationResult::TimedOut => self.timed_out += 1,
        }
        metrics.record_operations(source, result, 1);
    }
}
