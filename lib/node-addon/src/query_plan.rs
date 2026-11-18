use std::{collections::HashSet, sync::Arc};

use graphql_tools::parser::{query, schema};
use hive_router::{
    pipeline::{
        progressive_override::{RequestOverrideContext, StableOverrideContext},
        query_plan::calculate_cache_key,
    },
    query_planner::{
        ast::normalization::{error::NormalizationError, normalize_operation},
        graph::{PlannerOverrideContext, PERCENTAGE_SCALE_FACTOR},
        planner::{
            plan_nodes::planning::{FetchNode, PlanNode},
            Planner, PlannerError,
        },
        utils::{cancellation::CancellationToken, parsing::safe_parse_operation},
    },
};
use napi::{Task, Unknown};
use serde_json::{json, Value};

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
    #[error("Failed to serialize query plan: {0}")]
    Serialize(#[from] serde_json::Error),
}

impl From<QueryPlanError> for napi::Error {
    fn from(value: QueryPlanError) -> Self {
        napi::Error::from_reason(value.to_string())
    }
}

pub fn compute_cache_key(
    planner: &Planner,
    query: &str,
    operation_name: Option<&str>,
    active_labels: HashSet<String>,
    percentage_value: f64,
) -> core::result::Result<String, QueryPlanError> {
    let parsed_operation = safe_parse_operation(query)?;

    let normalized_operation =
        normalize_operation(&planner.supergraph, &parsed_operation, operation_name)?;

    let request_override_context = RequestOverrideContext {
        active_flags: active_labels,
        percentage_value: (percentage_value * (PERCENTAGE_SCALE_FACTOR as f64)) as u64,
    };

    let stable_override_context =
        StableOverrideContext::new(&planner.supergraph, &request_override_context);

    let operation_hash = normalized_operation.operation.hash();
    let cache_key = calculate_cache_key(operation_hash, &stable_override_context);

    Ok(cache_key.to_string())
}

pub fn query_plan(
    planner: &Planner,
    query: &str,
    operation_name: Option<&str>,
    active_labels: HashSet<String>,
    percentage_value: f64,
    cancellation_token: &CancellationToken,
) -> core::result::Result<Value, QueryPlanError> {
    let parsed_operation = safe_parse_operation(query)?;
    let normalized_operation =
        normalize_operation(&planner.supergraph, &parsed_operation, operation_name)?;

    let request_override_context = PlannerOverrideContext::new(
        active_labels,
        (percentage_value * (PERCENTAGE_SCALE_FACTOR as f64)) as u64,
    );

    let plan = planner.plan_from_normalized_operation(
        &normalized_operation.operation,
        request_override_context,
        cancellation_token,
    )?;

    let mut json = serde_json::to_value(&plan)?;
    if let Some(node) = plan.node() {
        add_fetch_ids(node, &mut json["node"]);
    }
    Ok(json)
}

/// Adds `id` and `dependsOn` to every fetch of the serialized plan `json`, and `completes` to
/// every batch fetch, so JS can start each fetch once the fetches it depends on are done. The
/// router's own plan serialization skips them, so they're added here, only for the addon.
/// `json` must be `node` serialized.
fn add_fetch_ids(node: &PlanNode, json: &mut Value) {
    match node {
        PlanNode::Fetch(fetch) => add_fetch_node_ids(fetch, json),
        PlanNode::BatchFetch(batch) => {
            json["id"] = json!(batch.id);
            json["dependsOn"] = json!(batch.depends_on.as_slice());
            json["completes"] = json!(batch.completes.as_slice());
        }
        PlanNode::Sequence(sequence) => add_fetch_ids_to_all(&sequence.nodes, &mut json["nodes"]),
        PlanNode::Parallel(parallel) => add_fetch_ids_to_all(&parallel.nodes, &mut json["nodes"]),
        PlanNode::Flatten(flatten) => add_fetch_ids(&flatten.node, &mut json["node"]),
        PlanNode::Subscription(subscription) => {
            add_fetch_node_ids(&subscription.primary, &mut json["primary"])
        }
        PlanNode::Defer(defer) => {
            if let Some(node) = &defer.primary.node {
                add_fetch_ids(node, &mut json["primary"]["node"]);
            }
            for (index, deferred) in defer.deferred.iter().enumerate() {
                if let Some(node) = &deferred.node {
                    add_fetch_ids(node, &mut json["deferred"][index]["node"]);
                }
            }
        }
    }
}

fn add_fetch_node_ids(fetch: &FetchNode, json: &mut Value) {
    json["id"] = json!(fetch.id);
    json["dependsOn"] = json!(fetch.depends_on.as_slice());
}

fn add_fetch_ids_to_all(nodes: &[PlanNode], json: &mut Value) {
    if let Some(json) = json.as_array_mut() {
        for (node, json) in nodes.iter().zip(json) {
            add_fetch_ids(node, json);
        }
    }
}

pub struct QueryPlanTask<'a> {
    pub planner: &'a Planner,
    pub query: String,
    pub operation_name: Option<String>,
    pub active_labels: HashSet<String>,
    pub percentage_value: f64,
    pub cancellation_token: Arc<CancellationToken>,
}

impl<'a> Task for QueryPlanTask<'a> {
    type Output = Value;
    type JsValue = Unknown<'a>;

    fn compute(&mut self) -> Result<Self::Output, napi::Error> {
        Ok(query_plan(
            self.planner,
            &self.query,
            self.operation_name.as_deref(),
            std::mem::take(&mut self.active_labels),
            self.percentage_value,
            &self.cancellation_token,
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
