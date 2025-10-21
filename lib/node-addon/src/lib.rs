#![deny(clippy::all)]

use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::planner::Planner;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use hive_router_query_planner::utils::parsing::{parse_schema, safe_parse_operation};

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;

#[napi]
pub struct QueryPlanner {
    planner: Arc<Planner>,
}

// TODO: Did not find struct `QueryPlanner` parsed before expand #[napi] for impl?
//       fixed in vscode with `"rust-analyzer.procMacro.ignored": { "napi-derive": ["napi"] }`
#[napi]
impl QueryPlanner {
    #[napi(constructor)]
    pub fn new(supergraph_sdl: String) -> Result<Self> {
        let parsed_supergraph = parse_schema(&supergraph_sdl);

        let planner = Planner::new_from_supergraph(&parsed_supergraph).map_err(|err| {
            napi::Error::from_reason(format!("Failed to create query planner: {}", err))
        })?;

        Ok(QueryPlanner {
            planner: Arc::new(planner),
        })
    }

    // queryplan located in query-plan.d.ts and will be merged with index.d.ts on build
    // because of napi-rs limitations, the queryplan from hive-query-planner cannot be used
    #[napi(ts_return_type = "Promise<QueryPlan>")]
    pub async fn plan(
        &self,
        query: String,
        operation_name: Option<String>,
    ) -> Result<serde_json::Value> {
        let planner = Arc::clone(&self.planner);

        tokio::task::spawn_blocking(move || {
            let parsed_operation = safe_parse_operation(&query)
                .map_err(|e| napi::Error::from_reason(format!("Failed to parse query: {}", e)))?;

            let normalized_operation = normalize_operation(
                &planner.supergraph,
                &parsed_operation,
                operation_name.as_deref(),
            )
            .map_err(|e| {
                napi::Error::from_reason(format!("Failed to normalize operation: {}", e))
            })?;

            // TODO: actually use the cacnellation token
            let cancellation_token = CancellationToken::new();

            let query_plan = planner
                .plan_from_normalized_operation(
                    &normalized_operation.operation,
                    Default::default(),
                    &cancellation_token,
                )
                .map_err(|e| napi::Error::from_reason(format!("Failed to plan query: {}", e)))?;

            serde_json::to_value(&query_plan).map_err(|e| {
                napi::Error::from_reason(format!("Failed to serialize query plan: {}", e))
            })
        })
        .await
        .map_err(|e| napi::Error::from_reason(format!("Task error: {}", e)))?
    }
}
