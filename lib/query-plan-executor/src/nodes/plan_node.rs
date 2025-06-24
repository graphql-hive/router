use futures::future::BoxFuture;
use query_planner::planner::plan_nodes::PlanNode;
use serde_json::Value;

use crate::{
    execution_context::ExecutionContext, execution_result::ExecutionResult,
    nodes::condition_node::ExecutableConditionNode, nodes::fetch_node::ExecutableFetchNode,
    nodes::flatten_node::ExecutableFlattenNode, nodes::parallel_node::ExecutableParallelNode,
    nodes::sequence_node::ExecutableSequenceNode,
};

pub trait ExecutablePlanNode {
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext,
    ) -> BoxFuture<'a, ExecutionResult>;
}

impl ExecutablePlanNode for PlanNode {
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext<'_>,
    ) -> BoxFuture<'a, ExecutionResult> {
        match self {
            PlanNode::Fetch(node) => node.execute(root, path, ctx),
            PlanNode::Flatten(node) => node.execute(root, path, ctx),
            PlanNode::Parallel(node) => node.execute(root, path, ctx),
            PlanNode::Sequence(node) => node.execute(root, path, ctx),
            PlanNode::Condition(node) => node.execute(root, path, ctx),
            _ => {
                unimplemented!("PlanNode type not implemented: {:?}", self);
            } // Add other plan nodes as needed
        }
    }
}
