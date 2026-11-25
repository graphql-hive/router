//! Plans known operations against a new supergraph before the router switches to it.
//!
//! Only the plan cache gets warmed. Its key comes from the normalized operation itself,
//! so a warmed plan is right no matter how a request later arrives at that operation.
//! The parse, validation and normalization caches are keyed by what the client sent and
//! what plugins did to it, which warmup can't reproduce, so those stay cold. They're cheap.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::config::cache::{CacheWarmupConfig, CacheWarmupHiveConfig};
use crate::config::HiveRouterConfig;
use crate::consts::ROUTER_VERSION;
use crate::executor::hooks::on_cache_warmup::{
    CacheWarmupDocument, CacheWarmupOperation, OnCacheWarmupHookPayload,
};
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
    hive: Option<HiveSlowOperations>,
    plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
}

impl CacheWarmup {
    /// `None` when warmup is turned off.
    pub fn from_config(
        router_config: &HiveRouterConfig,
        plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
    ) -> Result<Option<Self>, String> {
        let config: &CacheWarmupConfig = &router_config.cache.warmup;
        if !config.enabled {
            return Ok(None);
        }
        Ok(Some(Self {
            timeout: config.timeout,
            max_tokens: router_config.limits.max_tokens.as_ref().map(|cfg| cfg.n),
            hive: config
                .hive
                .as_ref()
                .map(HiveSlowOperations::try_new)
                .transpose()?,
            plugins,
        }))
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

        // slowest first, they're the ones worth the time budget the most
        if let Some(hive) = &self.hive {
            match tokio::time::timeout_at(deadline.into(), hive.fetch()).await {
                Ok(Ok(hive_operations)) => operations.extend(hive_operations),
                Ok(Err(error)) => {
                    warn!(target: targets::SUPERGRAPH, %error, "failed to fetch slow operations from Hive Console for cache warmup")
                }
                Err(_) => {
                    warn!(target: targets::SUPERGRAPH, "fetching slow operations from Hive Console took the whole cache warmup timeout")
                }
            }
        }

        if let Some(previous) = previous {
            for (_, payload) in previous.normalize_cache.iter() {
                operations.push(CacheWarmupOperation::from_document(
                    payload.source_document.clone(),
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
            let document = match operation.document {
                CacheWarmupDocument::Parsed(document) => document,
                CacheWarmupDocument::Text(query) => {
                    let parsed = match max_tokens {
                        Some(limit) => safe_parse_operation_with_token_limit(&query, limit),
                        None => safe_parse_operation(&query),
                    };
                    Arc::new(parsed.map_err(|error| error.to_string())?)
                }
            };
            let normalized = normalize_document(
                &supergraph,
                &document,
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

/// Fetches the operations Hive Console found slowest to plan.
///
/// The endpoint is `<cdn endpoint>/slow-operations`, answering with
/// `{ "operations": [{ "query": "...", "operationName": "..." }] }`, slowest first.
struct HiveSlowOperations {
    client: reqwest::Client,
    url: String,
    key: String,
}

#[derive(Deserialize)]
struct HiveSlowOperationsResponse {
    operations: Vec<HiveSlowOperation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HiveSlowOperation {
    query: String,
    #[serde(default)]
    operation_name: Option<String>,
}

impl HiveSlowOperations {
    fn try_new(config: &CacheWarmupHiveConfig) -> Result<Self, String> {
        let endpoint = std::env::var("HIVE_CDN_ENDPOINT")
            .ok()
            // the env var can hold a comma-separated list, the first one is enough here
            .and_then(|value| value.split(',').next().map(|s| s.trim().to_string()))
            .filter(|value| !value.is_empty())
            .or_else(|| config.endpoint.clone())
            .ok_or("cache warmup from Hive Console needs 'cache.warmup.hive.endpoint' or the HIVE_CDN_ENDPOINT environment variable")?;
        let key = std::env::var("HIVE_CDN_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| config.key.clone())
            .ok_or("cache warmup from Hive Console needs 'cache.warmup.hive.key' or the HIVE_CDN_KEY environment variable")?;

        let base = endpoint.trim_end_matches('/');
        // people paste the same endpoint they use for the supergraph
        let base = base.strip_suffix("/supergraph").unwrap_or(base);

        let client = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .user_agent(format!("hive-router/{ROUTER_VERSION}"))
            .build()
            .map_err(|error| error.to_string())?;

        Ok(Self {
            client,
            url: format!("{base}/slow-operations"),
            key,
        })
    }

    async fn fetch(&self) -> Result<Vec<CacheWarmupOperation>, String> {
        let response = self
            .client
            .get(&self.url)
            .header("X-Hive-CDN-Key", &self.key)
            .send()
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|error| error.to_string())?;
        let body = response.bytes().await.map_err(|error| error.to_string())?;
        let parsed: HiveSlowOperationsResponse =
            serde_json::from_slice(&body).map_err(|error| error.to_string())?;
        Ok(parsed
            .operations
            .into_iter()
            .map(|operation| {
                CacheWarmupOperation::from_text(
                    operation.query,
                    operation.operation_name,
                    CacheWarmupSource::HiveConsole,
                )
            })
            .collect())
    }
}
