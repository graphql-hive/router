use futures::future::BoxFuture;
use query_planner::planner::plan_nodes::FlattenNode;
use serde_json::Value;
use tracing::instrument;

use crate::{
    execution_context::ExecutionContext, execution_result::ExecutionResult,
    nodes::plan_node::ExecutablePlanNode,
};

pub trait ExecutableFlattenNode {
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext,
    ) -> BoxFuture<'a, ExecutionResult>;
}

impl ExecutableFlattenNode for FlattenNode {
    #[instrument(level = "debug", skip_all, name = "FlattenNode::execute", fields(
          path = ?self.path
      ))]
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext<'_>,
    ) -> BoxFuture<'a, ExecutionResult> {
        let mut new_path = path
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<String>>();
        new_path.extend(self.path.clone());
        self.node.execute(root, new_path, ctx)
    }
}
