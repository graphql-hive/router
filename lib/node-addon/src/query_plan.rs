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
            plan_nodes::planning::{
                BatchFetchNode, DeferDependency, FetchId, FetchNode, FlattenNodePath, PlanNode,
                QueryPlan,
            },
            Planner, PlannerError,
        },
        utils::{cancellation::CancellationToken, parsing::safe_parse_operation},
    },
};
use napi::{Task, Unknown};
use serde::{Serialize, Serializer};

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
) -> core::result::Result<QueryPlan, QueryPlanError> {
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

    Ok(plan)
}

/// The plan as the addon gives it to JS. Serializes like the router's own plan, but every
/// fetch also has its `id` and `dependsOn`, and every batch fetch its `completes`, so JS can
/// start each fetch once the fetches it depends on are done. The router's own serialization
/// skips them, so the addon serializes the nodes holding fetches itself.
pub struct JsQueryPlan<'a>(pub &'a QueryPlan);

impl Serialize for JsQueryPlan<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Plan<'a> {
            kind: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            node: Option<JsPlanNode<'a>>,
        }
        Plan {
            kind: self.0.kind,
            node: self.0.node().map(JsPlanNode),
        }
        .serialize(serializer)
    }
}

struct JsPlanNode<'a>(&'a PlanNode);

struct JsPlanNodes<'a>(&'a [PlanNode]);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsFetchNode<'a> {
    #[serde(flatten)]
    fetch: &'a FetchNode,
    id: FetchId,
    depends_on: &'a [FetchId],
}

impl<'a> From<&'a FetchNode> for JsFetchNode<'a> {
    fn from(fetch: &'a FetchNode) -> Self {
        Self {
            fetch,
            id: fetch.id,
            depends_on: &fetch.depends_on,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsBatchFetchNode<'a> {
    #[serde(flatten)]
    batch: &'a BatchFetchNode,
    id: FetchId,
    depends_on: &'a [FetchId],
    completes: &'a [FetchId],
}

#[derive(Serialize)]
struct JsDeferPrimary<'a> {
    subselection: &'a Option<String>,
    node: Option<JsPlanNode<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsDeferredNode<'a> {
    depends: &'a [DeferDependency],
    #[serde(skip_serializing_if = "Option::is_none")]
    label: &'a Option<String>,
    query_path: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    subselection: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node: Option<JsPlanNode<'a>>,
}

// same shape as the router's `PlanNode`, only with ids on the fetches
#[derive(Serialize)]
#[serde(tag = "kind")]
enum JsTaggedPlanNode<'a> {
    Fetch(JsFetchNode<'a>),
    BatchFetch(JsBatchFetchNode<'a>),
    Sequence {
        nodes: JsPlanNodes<'a>,
    },
    Parallel {
        nodes: JsPlanNodes<'a>,
    },
    Flatten {
        path: &'a FlattenNodePath,
        node: JsPlanNode<'a>,
    },
    Subscription {
        primary: JsFetchNode<'a>,
    },
    Defer {
        primary: JsDeferPrimary<'a>,
        deferred: Vec<JsDeferredNode<'a>>,
    },
}

impl Serialize for JsPlanNode<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            PlanNode::Fetch(fetch) => JsTaggedPlanNode::Fetch(fetch.as_ref().into()),
            PlanNode::BatchFetch(batch) => JsTaggedPlanNode::BatchFetch(JsBatchFetchNode {
                batch,
                id: batch.id,
                depends_on: &batch.depends_on,
                completes: &batch.completes,
            }),
            PlanNode::Sequence(sequence) => JsTaggedPlanNode::Sequence {
                nodes: JsPlanNodes(&sequence.nodes),
            },
            PlanNode::Parallel(parallel) => JsTaggedPlanNode::Parallel {
                nodes: JsPlanNodes(&parallel.nodes),
            },
            PlanNode::Flatten(flatten) => JsTaggedPlanNode::Flatten {
                path: &flatten.path,
                node: JsPlanNode(&flatten.node),
            },
            PlanNode::Subscription(subscription) => JsTaggedPlanNode::Subscription {
                primary: (&subscription.primary).into(),
            },
            PlanNode::Defer(defer) => JsTaggedPlanNode::Defer {
                primary: JsDeferPrimary {
                    subselection: &defer.primary.subselection,
                    node: defer.primary.node.as_deref().map(JsPlanNode),
                },
                deferred: defer
                    .deferred
                    .iter()
                    .map(|deferred| JsDeferredNode {
                        depends: &deferred.depends,
                        label: &deferred.label,
                        query_path: &deferred.query_path,
                        subselection: &deferred.subselection,
                        node: deferred.node.as_deref().map(JsPlanNode),
                    })
                    .collect(),
            },
        }
        .serialize(serializer)
    }
}

impl Serialize for JsPlanNodes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(JsPlanNode))
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
    type Output = QueryPlan;
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
        env.to_js_value(&JsQueryPlan(&output))
    }
}
