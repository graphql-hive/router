#![deny(clippy::all)]

mod query_plan;
use hive_router_query_planner::graph::PERCENTAGE_SCALE_FACTOR;
use hive_router_query_planner::planner::Planner;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use hive_router_query_planner::utils::parsing::safe_parse_schema;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashSet;
use std::sync::Arc;

use crate::query_plan::{QueryPlanError, QueryPlanTask};

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
        let parsed_supergraph =
            safe_parse_schema(&supergraph_sdl).map_err(QueryPlanError::SchemaParse)?;

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
        let cancellation_token = Arc::new(CancellationToken::new());
        if let Some(signal) = signal {
            let cancellation_token_clone = Arc::clone(&cancellation_token);
            signal.on_abort(move || {
                cancellation_token_clone.cancel();
            });
        }
        let query_plan = query_plan::query_plan(
            &self.planner,
            query.as_str(),
            operation_name.as_deref(),
            active_labels,
            percentage_value,
            &cancellation_token,
        )?;

        serde_json::to_value(&query_plan).map_err(|err| {
            napi::Error::from_reason(format!("Failed to serialize query plan: {}", err))
        })
    }

    #[napi(ts_return_type = "Promise<QueryPlan>")]
    pub fn plan_async<'a>(
        &'a self,
        query: String,
        operation_name: Option<String>,
        active_labels: HashSet<String>,
        percentage_value: f64,
        signal: Option<AbortSignal>,
    ) -> AsyncTask<QueryPlanTask<'a>> {
        AsyncTask::with_optional_signal(
            QueryPlanTask {
                planner: &self.planner,
                query,
                operation_name,
                active_labels,
                percentage_value,
            },
            signal,
        )
    }
}
