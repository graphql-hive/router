//! Plans known operations against a new supergraph before the router switches to it.
//!
//! Only the plan cache gets warmed. Its key comes from the normalized operation itself,
//! so a warmed plan is right no matter how a request later arrives at that operation.
//! The parse, validation and normalization caches are keyed by what the client sent and
//! what plugins did to it, which warmup can't reproduce, so those stay cold. They're cheap.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use crate::config::cache::{CacheWarmupOrder, CacheWarmupOrderPreset};
use crate::config::HiveRouterConfig;
use crate::executor::hooks::on_cache_warmup::{CacheWarmupOperation, OnCacheWarmupHookPayload};
use crate::executor::hooks::on_supergraph_load::SupergraphSnapshot;
use crate::executor::plugin_trait::RouterPluginBoxed;
use crate::pipeline::normalize::{millis_since_start, normalize_document};
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
use crate::vrl::expressions::{ExecutableProgram, VrlProgram, VrlValue};

pub(crate) struct CacheWarmup {
    timeout: Duration,
    order: Order,
    max_tokens: Option<usize>,
    plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
}

impl CacheWarmup {
    /// `None` when warmup is turned off. Fails when the order expression doesn't compile.
    pub fn from_config(
        router_config: &HiveRouterConfig,
        plugins: Option<Arc<Vec<RouterPluginBoxed>>>,
    ) -> Result<Option<Self>, String> {
        let config = &router_config.cache.warmup;
        if !config.enabled {
            return Ok(None);
        }
        let order = match &config.order {
            CacheWarmupOrder::Preset(preset) => Order::Preset(*preset),
            CacheWarmupOrder::Expression { expression } => {
                Order::Expression(Box::new(compile_order_expression(expression).map_err(
                    |error| format!("invalid 'cache.warmup.order' expression:\n{error}"),
                )?))
            }
        };
        Ok(Some(Self {
            timeout: config.timeout,
            order,
            max_tokens: router_config.limits.max_tokens.as_ref().map(|cfg| cfg.n),
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

        if let Some(previous) = previous {
            let previous_operations: Vec<_> = previous
                .normalize_cache
                .iter()
                .map(|(_, payload)| {
                    let usage = &payload.usage;
                    let stats = UsageStats {
                        operation_type: payload.operation_kind.as_str(),
                        hits: usage.hits(),
                        last_used_ms: usage.last_used_ms(),
                        plan_duration: usage.plan_duration(),
                    };
                    let operation = CacheWarmupOperation::from_text(
                        payload.source_query.clone(),
                        payload.requested_operation_name.clone(),
                        CacheWarmupSource::PreviousSupergraph,
                    );
                    (stats, operation)
                })
                .collect();
            operations.extend(self.order.apply(previous_operations));
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
        let mut remaining = operations.into_iter().peekable();
        // nothing new starts once the deadline has passed
        while let Some(operation) = remaining.next_if(|_| Instant::now() < deadline) {
            let source = operation.source;
            let result = self
                .warm_one(operation, supergraph, runtime, deadline)
                .await;
            counts.record(metrics, source, result);
        }

        // the ones that never got their turn, one line for all of them, there can be a thousand
        let mut not_started = 0;
        for operation in remaining {
            not_started += 1;
            counts.record(
                metrics,
                operation.source,
                CacheWarmupOperationResult::TimedOut,
            );
        }
        if not_started > 0 {
            warn!(
                target: targets::SUPERGRAPH,
                timeout_ms = self.timeout.as_millis(),
                not_started,
                "plan cache warmup ran out of time, switching to the new supergraph with what got planned"
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
        let query = operation.query.clone();

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
            Ok(Err(error)) if Instant::now() >= deadline => {
                warn!(
                    target: targets::SUPERGRAPH,
                    operation_name = ?operation_name,
                    query = %query,
                    %error,
                    "plan cache warmup ran out of time while planning an operation"
                );
                CacheWarmupOperationResult::TimedOut
            }
            Ok(Err(error)) => {
                warn!(
                    target: targets::SUPERGRAPH,
                    operation_name = ?operation_name,
                    query = %query,
                    %error,
                    "failed to warm up the plan of an operation"
                );
                CacheWarmupOperationResult::Failed
            }
            Err(error) => {
                warn!(
                    target: targets::SUPERGRAPH,
                    operation_name = ?operation_name,
                    query = %query,
                    %error,
                    "failed to warm up the plan of an operation, the planning task did not finish"
                );
                CacheWarmupOperationResult::Failed
            }
        }
    }
}

enum Order {
    Preset(CacheWarmupOrderPreset),
    Expression(Box<VrlProgram>),
}

struct UsageStats {
    operation_type: &'static str,
    hits: u64,
    /// from `millis_since_start`, 0 when never used
    last_used_ms: u64,
    plan_duration: Duration,
}

impl Order {
    /// Highest first. Operations the expression scores `null` are left out.
    fn apply<T>(&self, mut operations: Vec<(UsageStats, T)>) -> Vec<T> {
        match self {
            Order::Preset(preset) => {
                sort_by_preset(*preset, &mut operations);
                operations
                    .into_iter()
                    .map(|(_, operation)| operation)
                    .collect()
            }
            Order::Expression(program) => sort_by_expression(program, operations),
        }
    }
}

/// Ties fall back to the other two numbers, so the order holds between runs.
fn sort_by_preset<T>(preset: CacheWarmupOrderPreset, operations: &mut [(UsageStats, T)]) {
    operations.sort_by_key(|(stats, _)| {
        let plan_us = u64::try_from(stats.plan_duration.as_micros()).unwrap_or(u64::MAX);
        let (hits, last_used) = (stats.hits, stats.last_used_ms);
        std::cmp::Reverse(match preset {
            CacheWarmupOrderPreset::SlowestToPlan => (plan_us, hits, last_used),
            CacheWarmupOrderPreset::MostUsed => (hits, last_used, plan_us),
            CacheWarmupOrderPreset::RecentlyUsed => (last_used, hits, plan_us),
        })
    });
}

fn sort_by_expression<T>(program: &VrlProgram, operations: Vec<(UsageStats, T)>) -> Vec<T> {
    let now_ms = millis_since_start();
    let mut excluded = 0;
    let mut errors = 0;
    let mut first_error = None;

    let mut scored: Vec<(f64, u64, T)> = Vec::with_capacity(operations.len());
    for (stats, operation) in operations {
        let score = match program.execute(expression_input(&stats, now_ms)) {
            Ok(VrlValue::Null) => {
                excluded += 1;
                continue;
            }
            Ok(VrlValue::Integer(score)) => score as f64,
            Ok(VrlValue::Float(score)) => score.into_inner(),
            Ok(other) => {
                errors += 1;
                first_error.get_or_insert_with(|| {
                    format!("expected a number or null, got {}", other.kind_str())
                });
                f64::NEG_INFINITY
            }
            Err(error) => {
                errors += 1;
                first_error.get_or_insert_with(|| error.to_string());
                f64::NEG_INFINITY
            }
        };
        scored.push((score, stats.hits, operation));
    }

    if errors > 0 {
        warn!(
            target: targets::SUPERGRAPH,
            errors,
            first_error = first_error.unwrap_or_default(),
            "the 'cache.warmup.order' expression failed for some operations, they go last"
        );
    }
    if excluded > 0 {
        info!(
            target: targets::SUPERGRAPH,
            excluded,
            "the 'cache.warmup.order' expression left some operations out of warmup"
        );
    }

    // NaN and errors end up last, ties go to the more used one
    scored.sort_by(|(a, a_hits, _), (b, b_hits, _)| {
        let a = if a.is_nan() { f64::NEG_INFINITY } else { *a };
        let b = if b.is_nan() { f64::NEG_INFINITY } else { *b };
        b.total_cmp(&a).then(b_hits.cmp(a_hits))
    });
    scored
        .into_iter()
        .map(|(_, _, operation)| operation)
        .collect()
}

/// Compiles against the exact shape of `expression_input`, so `.operation.hits * 2`
/// type-checks without `to_int!` around every field.
fn compile_order_expression(source: &str) -> Result<VrlProgram, String> {
    use vrl::compiler::{compile_with_external, state::ExternalEnv, CompileConfig};
    use vrl::value::kind::{Field, Kind};

    let operation = Kind::object(BTreeMap::from([
        (Field::from("type"), Kind::bytes()),
        (Field::from("hits"), Kind::integer()),
        (Field::from("last_used_ms_ago"), Kind::integer().or_null()),
        (Field::from("plan_duration_ms"), Kind::float()),
    ]));
    let input = Kind::object(BTreeMap::from([(Field::from("operation"), operation)]));
    let external = ExternalEnv::new_with_kind(input, Kind::object(BTreeMap::<Field, Kind>::new()));

    compile_with_external(
        source,
        &vrl::stdlib::all(),
        &external,
        CompileConfig::default(),
    )
    .map(|compiled| compiled.program)
    .map_err(|diagnostics| vrl::diagnostic::Formatter::new(source, diagnostics).to_string())
}

/// What the order expression sees as `.operation`.
fn expression_input(stats: &UsageStats, now_ms: u64) -> VrlValue {
    // VRL integers are i64, numbers past it would take 292 million years to reach
    let int = |value: u64| VrlValue::Integer(i64::try_from(value).unwrap_or(i64::MAX));
    let last_used_ms_ago = if stats.last_used_ms == 0 {
        VrlValue::Null
    } else {
        int(now_ms.saturating_sub(stats.last_used_ms))
    };
    VrlValue::Object(BTreeMap::from([(
        "operation".into(),
        VrlValue::Object(BTreeMap::from([
            ("type".into(), VrlValue::from(stats.operation_type)),
            ("hits".into(), int(stats.hits)),
            ("last_used_ms_ago".into(), last_used_ms_ago),
            (
                "plan_duration_ms".into(),
                VrlValue::from_f64_or_zero(stats.plan_duration.as_secs_f64() * 1000.0),
            ),
        ])),
    )]))
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

#[cfg(test)]
mod tests {
    use super::*;

    // (name, type, hits, last used ms, planning ms)
    const OPERATIONS: [(&str, &str, u64, u64, u64); 4] = [
        ("hot", "query", 5, 300, 1),
        ("recent", "query", 1, 900, 2),
        ("slow", "query", 2, 100, 40),
        ("write", "mutation", 3, 200, 30),
    ];

    fn ordered(order: Order) -> Vec<&'static str> {
        let operations = OPERATIONS
            .iter()
            .map(|(name, operation_type, hits, last_used_ms, plan_ms)| {
                let stats = UsageStats {
                    operation_type,
                    hits: *hits,
                    last_used_ms: *last_used_ms,
                    plan_duration: Duration::from_millis(*plan_ms),
                };
                (stats, *name)
            })
            .collect();
        order.apply(operations)
    }

    fn expression(source: &str) -> Order {
        Order::Expression(Box::new(compile_order_expression(source).unwrap()))
    }

    #[test]
    fn presets_sort_highest_first() {
        use CacheWarmupOrderPreset::*;
        assert_eq!(
            ordered(Order::Preset(SlowestToPlan)),
            ["slow", "write", "recent", "hot"]
        );
        assert_eq!(
            ordered(Order::Preset(MostUsed)),
            ["hot", "write", "slow", "recent"]
        );
        assert_eq!(
            ordered(Order::Preset(RecentlyUsed)),
            ["recent", "hot", "write", "slow"]
        );
    }

    #[test]
    fn expression_scores_and_null_leaves_operations_out() {
        let order = expression(
            r#"if .operation.type == "mutation" { null } else { .operation.plan_duration_ms * .operation.hits }"#,
        );
        // slow 40*2=80, recent 2*1=2, hot 1*5=5, the mutation is gone
        assert_eq!(ordered(order), ["slow", "hot", "recent"]);
    }

    #[test]
    fn expression_errors_go_last_instead_of_failing_warmup() {
        let order = expression(r#"if .operation.hits > 2 { "oops" } else { .operation.hits }"#);
        // recent (1) and slow (2) score, hot (5) and write (3) error and keep their hits order
        assert_eq!(ordered(order), ["slow", "recent", "hot", "write"]);
    }

    #[test]
    fn expression_sees_time_since_last_use() {
        let now = millis_since_start();
        let stats = UsageStats {
            operation_type: "query",
            hits: u64::MAX,
            last_used_ms: 0,
            plan_duration: Duration::ZERO,
        };
        let input = expression_input(&stats, now);
        let operation = &input.as_object().unwrap()["operation"];
        // never used, and a count past i64 saturates instead of wrapping
        assert_eq!(
            operation.as_object().unwrap()["last_used_ms_ago"],
            VrlValue::Null
        );
        assert_eq!(
            operation.as_object().unwrap()["hits"],
            VrlValue::Integer(i64::MAX)
        );
    }
}

#[cfg(test)]
mod compile_tests {
    use super::*;

    #[test]
    fn unknown_fields_fail_to_compile() {
        assert!(compile_order_expression(".operation.hitz * 2").is_err());
        assert!(compile_order_expression(".operation.hits * 2").is_ok());
    }
}
