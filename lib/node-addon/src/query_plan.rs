use std::collections::HashSet;

use graphql_tools::parser::{query, schema};
use hive_router_query_planner::{
    ast::normalization::{error::NormalizationError, normalize_operation},
    graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR},
    planner::{plan_nodes::QueryPlan, Planner, PlannerError},
    utils::{cancellation::CancellationToken, parsing::safe_parse_operation},
};
use napi::{Task, Unknown};

#[derive(Debug, thiserror::Error)]
pub enum QueryPlanError {
    #[error("Failed to parse supergraph SDL: {0}")]
    SchemaParse(#[from] schema::ParseError),
    #[error("Failed to parse query: {0}")]
    QueryParse(#[from] query::ParseError),
    #[error("Failed to normalize operation: {0}")]
    Normalization(#[from] NormalizationError),
    #[error("Failed to plan query: {0}")]
    Planner(#[from] PlannerError),
}

impl From<QueryPlanError> for napi::Error {
    fn from(value: QueryPlanError) -> Self {
        napi::Error::from_reason(value.to_string())
    }
}

pub fn query_plan(
    planner: &Planner,
    query: &str,
    operation_name: Option<&str>,
    active_labels: HashSet<String>,
    percentage_value: f64,
    cancellation_token: &CancellationToken,
) -> core::result::Result<QueryPlan, QueryPlanError> {
    let parsed_operation = safe_parse_operation(query)?;

    let normalized_operation =
        normalize_operation(&planner.supergraph, &parsed_operation, operation_name)?;

    let request_override_context = PlannerOverrideContext::new(
        active_labels,
        (percentage_value * (PERCENTAGE_SCALE_FACTOR as f64)) as u64,
    );

    Ok(planner.plan_from_normalized_operation(
        &normalized_operation.operation,
        request_override_context,
        cancellation_token,
    )?)
}

pub struct QueryPlanTask<'a> {
    pub planner: &'a Planner,
    pub query: String,
    pub operation_name: Option<String>,
    pub active_labels: HashSet<String>,
    pub percentage_value: f64,
}

impl<'a> Task for QueryPlanTask<'a> {
    type Output = QueryPlan;
    type JsValue = Unknown<'a>;

    fn compute(&mut self) -> Result<Self::Output, napi::Error> {
        Ok(query_plan(
            self.planner,
            &self.query,
            self.operation_name.as_deref(),
            std::mem::take(&mut self.active_labels),
            self.percentage_value,
            &Default::default(), // TODO: Pass cancellation token from JS
        )?)
    }

    fn resolve(
        &mut self,
        env: napi::Env,
        output: Self::Output,
    ) -> Result<Self::JsValue, napi::Error> {
        env.to_js_value(&output)
    }
}
