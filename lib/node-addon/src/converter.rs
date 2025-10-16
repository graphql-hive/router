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
            source::PlanNode::Sequence(node) => PlanNode::Sequence(node.into()),
            source::PlanNode::Parallel(node) => PlanNode::Parallel(node.into()),
            source::PlanNode::Flatten(node) => PlanNode::Flatten(node.into()),
            source::PlanNode::Condition(node) => PlanNode::Condition(node.into()),
            source::PlanNode::Subscription(node) => PlanNode::Subscription(node.into()),
            source::PlanNode::Defer(node) => PlanNode::Defer(node.into()),
        }
    }
}

/// Convert from the query planner's SequenceNode to the node-addon's SequenceNode
impl From<source::SequenceNode> for SequenceNode {
    fn from(source_node: source::SequenceNode) -> Self {
        SequenceNode {
            nodes: source_node.nodes.into_iter().map(Into::into).collect(),
        }
    }
}

/// Convert from the query planner's ParallelNode to the node-addon's ParallelNode
impl From<source::ParallelNode> for ParallelNode {
    fn from(source_node: source::ParallelNode) -> Self {
        ParallelNode {
            nodes: source_node.nodes.into_iter().map(Into::into).collect(),
        }
    }
}

/// Convert from the query planner's FlattenNode to the node-addon's FlattenNode
impl From<source::FlattenNode> for FlattenNode {
    fn from(source_node: source::FlattenNode) -> Self {
        FlattenNode {
            path: source_node.path,
            node: Box::new((*source_node.node).into()),
        }
    }
}

/// Convert from the query planner's ConditionNode to the node-addon's ConditionNode
impl From<source::ConditionNode> for ConditionNode {
    fn from(source_node: source::ConditionNode) -> Self {
        ConditionNode {
            condition: source_node.condition,
            if_clause: source_node.if_clause.map(|node| Box::new((*node).into())),
            else_clause: source_node.else_clause.map(|node| Box::new((*node).into())),
        }
    }
}

/// Convert from the query planner's SubscriptionNode to the node-addon's SubscriptionNode
impl From<source::SubscriptionNode> for SubscriptionNode {
    fn from(source_node: source::SubscriptionNode) -> Self {
        SubscriptionNode {
            primary: Box::new((*source_node.primary).into()),
        }
    }
}

/// Convert from the query planner's DeferNode to the node-addon's DeferNode
impl From<source::DeferNode> for DeferNode {
    fn from(source_node: source::DeferNode) -> Self {
        DeferNode {
            primary: source_node.primary,
            deferred: source_node.deferred,
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
