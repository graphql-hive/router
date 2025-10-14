use hive_router_query_planner::{
    ast::{document::Document, selection_set::SelectionSet},
    planner::plan_nodes::{DeferPrimary, DeferredNode, FetchRewrite, FlattenNodePath},
    state::supergraph_state::OperationKind,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct QueryPlan {
    pub kind: String, // "QueryPlan"
    pub node: Option<PlanNode>,
}

#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum PlanNode {
    Sequence(SequenceNode),
    Parallel(ParallelNode),
    Flatten(FlattenNode),
    Condition(ConditionNode),
    Subscription(SubscriptionNode),
    Defer(DeferNode),
    Fetch(FetchNode),
}

#[derive(Serialize)]
pub struct SequenceNode {
    pub nodes: Vec<PlanNode>,
}

#[derive(Serialize)]
pub struct ParallelNode {
    pub nodes: Vec<PlanNode>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlattenNode {
    pub path: FlattenNodePath,
    pub node: Box<PlanNode>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionNode {
    pub condition: String,
    pub if_clause: Option<Box<PlanNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub else_clause: Option<Box<PlanNode>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionNode {
    pub primary: Box<PlanNode>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferNode {
    pub primary: DeferPrimary,
    pub deferred: Vec<DeferredNode>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchNode {
    pub service_name: String,
    pub variable_usages: Vec<String>,
    pub operation_kind: Option<OperationKind>,
    pub operation_name: Option<String>,
    pub operation: String,
    pub requires: Option<SelectionSet>,
    pub input_rewrites: Option<Vec<FetchRewrite>>,
    pub output_rewrites: Option<Vec<FetchRewrite>>,
    // we added this, everything else is the same as from plan_nodes.rs
    pub operation_document_node: Document,
}
