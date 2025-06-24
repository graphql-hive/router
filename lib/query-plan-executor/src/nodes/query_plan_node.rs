use futures::future::BoxFuture;
use query_planner::planner::plan_nodes::QueryPlan;
use serde_json::Value;

use crate::{
    execution_context::ExecutionContext, execution_result::ExecutionResult,
    nodes::plan_node::ExecutablePlanNode,
};

pub trait ExecutableQueryPlanNode {
    fn execute<'a>(&'a self, ctx: &'a ExecutionContext<'_>) -> BoxFuture<'a, ExecutionResult>;
}

impl ExecutableQueryPlanNode for QueryPlan {
    fn execute<'a>(&'a self, ctx: &'a ExecutionContext<'_>) -> BoxFuture<'a, ExecutionResult> {
        if let Some(node) = &self.node {
            node.execute(&Value::Null, vec![], ctx)
        } else {
            Box::pin(async move { ExecutionResult::default() })
        }
    }
}
