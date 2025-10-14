use hive_router_query_planner::planner::plan_nodes as source;

use crate::plan_nodes::{
    ConditionNode, DeferNode, FetchNode, FlattenNode, ParallelNode, PlanNode, QueryPlan,
    SequenceNode, SubscriptionNode,
};

/// Convert from the query planner's QueryPlan to the node-addon's QueryPlan
impl From<source::QueryPlan> for QueryPlan {
    fn from(source_plan: source::QueryPlan) -> Self {
        QueryPlan {
            kind: source_plan.kind,
            node: source_plan.node.map(Into::into),
        }
    }
}

/// Convert from the query planner's PlanNode to the node-addon's PlanNode
impl From<source::PlanNode> for PlanNode {
    fn from(source_node: source::PlanNode) -> Self {
        match source_node {
            source::PlanNode::Fetch(node) => PlanNode::Fetch(node.into()),
            source::PlanNode::Sequence(node) => PlanNode::Sequence(convert_sequence_node(node)),
            source::PlanNode::Parallel(node) => PlanNode::Parallel(convert_parallel_node(node)),
            source::PlanNode::Flatten(node) => PlanNode::Flatten(convert_flatten_node(node)),
            source::PlanNode::Condition(node) => PlanNode::Condition(convert_condition_node(node)),
            source::PlanNode::Subscription(node) => {
                PlanNode::Subscription(convert_subscription_node(node))
            }
            source::PlanNode::Defer(node) => PlanNode::Defer(convert_defer_node(node)),
        }
    }
}

/// Convert from the query planner's FetchNode to the node-addon's FetchNode
impl From<source::FetchNode> for FetchNode {
    fn from(source_node: source::FetchNode) -> Self {
        FetchNode {
            service_name: source_node.service_name,
            variable_usages: source_node
                .variable_usages
                .map(|set| set.into_iter().collect())
                .unwrap_or_default(),
            operation_kind: source_node.operation_kind,
            operation_name: source_node.operation_name,
            operation: source_node.operation.document_str,
            requires: source_node.requires,
            input_rewrites: source_node.input_rewrites,
            output_rewrites: source_node.output_rewrites,
            operation_document_node: source_node.operation.document,
        }
    }
}

/// Convert FlattenNode by recursively converting the inner node
fn convert_flatten_node(source_node: source::FlattenNode) -> FlattenNode {
    FlattenNode {
        path: source_node.path,
        node: Box::new((*source_node.node).into()),
    }
}

/// Convert ConditionNode by recursively converting if/else clauses
fn convert_condition_node(source_node: source::ConditionNode) -> ConditionNode {
    ConditionNode {
        condition: source_node.condition,
        if_clause: source_node.if_clause.map(|node| Box::new((*node).into())),
        else_clause: source_node.else_clause.map(|node| Box::new((*node).into())),
    }
}

/// Convert SubscriptionNode by recursively converting the primary node
fn convert_subscription_node(source_node: source::SubscriptionNode) -> SubscriptionNode {
    SubscriptionNode {
        primary: Box::new((*source_node.primary).into()),
    }
}

/// Convert SequenceNode by recursively converting all child nodes
fn convert_sequence_node(source_node: source::SequenceNode) -> SequenceNode {
    SequenceNode {
        nodes: source_node.nodes.into_iter().map(Into::into).collect(),
    }
}

/// Convert ParallelNode by recursively converting all child nodes
fn convert_parallel_node(source_node: source::ParallelNode) -> ParallelNode {
    ParallelNode {
        nodes: source_node.nodes.into_iter().map(Into::into).collect(),
    }
}

/// Convert DeferNode by recursively converting primary and deferred nodes
fn convert_defer_node(source_node: source::DeferNode) -> DeferNode {
    DeferNode {
        primary: source_node.primary,
        deferred: source_node.deferred,
    }
}
