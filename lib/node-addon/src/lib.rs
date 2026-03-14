#![deny(clippy::all)]

use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR};
use hive_router_query_planner::planner::Planner;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use hive_router_query_planner::utils::parsing::{safe_parse_operation, safe_parse_schema};

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashSet;
use std::sync::Arc;

#[napi]
pub struct QueryPlanner {
    planner: Planner,
}

// TODO: Did not find struct `QueryPlanner` parsed before expand #[napi] for impl?
//       fixed in vscode with `"rust-analyzer.procMacro.ignored": { "napi-derive": ["napi"] }`
#[napi]
impl QueryPlanner {
    #[napi(constructor)]
    pub fn new(supergraph_sdl: String) -> Result<Self> {
        let parsed_supergraph = safe_parse_schema(&supergraph_sdl).map_err(|e| {
            napi::Error::from_reason(format!("Failed to parse supergraph SDL: {}", e))
        })?;

        let planner = Planner::new_from_supergraph(&parsed_supergraph).map_err(|err| {
            napi::Error::from_reason(format!("Failed to create query planner: {}", err))
        })?;

        Ok(QueryPlanner { planner })
    }

    #[napi(getter)]
    pub fn consumer_schema(&self) -> String {
        self.planner.consumer_schema.document.to_string()
    }

    #[napi(getter)]
    pub fn override_labels(&self) -> HashSet<String> {
        self.planner.supergraph.progressive_overrides.flags.clone()
    }

    #[napi(getter)]
    pub fn override_percentages(&self) -> Vec<f64> {
        self.planner
            .supergraph
            .progressive_overrides
            .percentages
            .iter()
            .map(|p: &u64| (*p as f64) / (PERCENTAGE_SCALE_FACTOR as f64))
            .collect()
    }

    // queryplan located in query-plan.d.ts and will be merged with index.d.ts on build
    // because of napi-rs limitations, the queryplan from hive-query-planner cannot be used
    #[napi(ts_return_type = "QueryPlan")]
    pub fn plan(
        &self,
        query: String,
        operation_name: Option<String>,
        active_labels: HashSet<String>,
        percentage_value: f64,
        signal: Option<AbortSignal>,
    ) -> Result<serde_json::Value> {
        let parsed_operation = safe_parse_operation(&query)
            .map_err(|e| napi::Error::from_reason(format!("Failed to parse query: {}", e)))?;

        let normalized_operation = normalize_operation(
            &self.planner.supergraph,
            &parsed_operation,
            operation_name.as_deref(),
        )
        .map_err(|e| napi::Error::from_reason(format!("Failed to normalize operation: {}", e)))?;

        // TODO: actually use the cacnellation token
        let cancellation_token = Arc::new(CancellationToken::new());
        let cancellation_token_clone = Arc::clone(&cancellation_token);

        if let Some(signal) = signal {
            signal.on_abort(move || {
                cancellation_token_clone.cancel();
            });
        }

        let request_override_context = PlannerOverrideContext::new(
            active_labels,
            (percentage_value * (PERCENTAGE_SCALE_FACTOR as f64)) as u64,
        );

        let query_plan = self
            .planner
            .plan_from_normalized_operation(
                &normalized_operation.operation,
                request_override_context,
                &cancellation_token,
            )
            .map_err(|e| napi::Error::from_reason(format!("Failed to plan query: {}", e)))?;

        serde_json::to_value(&query_plan)
            .map_err(|e| napi::Error::from_reason(format!("Failed to serialize query plan: {}", e)))
    }
}
