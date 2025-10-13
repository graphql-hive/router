#![deny(clippy::all)]

use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::planner::Planner;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use hive_router_query_planner::utils::parsing::{parse_schema, safe_parse_operation};
use napi::bindgen_prelude::*;

#[macro_use]
extern crate napi_derive;

#[napi]
pub struct QueryPlanner {
    planner: Planner,
}

// TODO: cannot find function `execute_tokio_future` in module `napi::bindgen_prelude`
#[napi]
impl QueryPlanner {
    #[napi(constructor)]
    pub fn new(supergraph_sdl: String) -> Result<Self> {
        let parsed_supergraph = parse_schema(&supergraph_sdl);

        let planner = Planner::new_from_supergraph(&parsed_supergraph).map_err(|err| {
            napi::Error::from_reason(format!("Failed to create query planner: {}", err))
        })?;

        Ok(QueryPlanner { planner: planner })
    }

    #[napi]
    pub fn plan(&self, query: String, operation_name: Option<String>) -> Result<String> {
        let planner = &self.planner;

        let parsed_operation = safe_parse_operation(&query)
            .map_err(|e| napi::Error::from_reason(format!("Failed to parse query: {}", e)))?;

        let normalized_operation = normalize_operation(
            &planner.supergraph,
            &parsed_operation,
            operation_name.as_deref(),
        )
        .map_err(|e| napi::Error::from_reason(format!("Failed to normalize operation: {}", e)))?;

        // TODO: actually use the cacnellation token
        let cancellation_token = CancellationToken::new();

        let query_plan = planner
            .plan_from_normalized_operation(
                &normalized_operation.operation,
                Default::default(),
                &cancellation_token,
            )
            .map_err(|e| napi::Error::from_reason(format!("Failed to plan query: {}", e)))?;

        let json_string = serde_json::to_string(&query_plan)
            .map_err(|e| napi::Error::from_reason(format!("Failed to serialize plan: {}", e)))?;

        Ok(json_string)
    }
}
