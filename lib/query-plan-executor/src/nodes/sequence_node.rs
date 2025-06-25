use std::collections::BTreeMap;

use futures::future::BoxFuture;
use query_planner::planner::plan_nodes::SequenceNode;
use serde_json::Value;
use tracing::instrument;

use crate::{
    deep_merge::DeepMerge, execution_context::ExecutionContext, execution_result::ExecutionResult,
    nodes::plan_node::ExecutablePlanNode,
};

pub trait ExecutableSequenceNode {
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext,
    ) -> BoxFuture<'a, ExecutionResult>;
}

impl ExecutableSequenceNode for SequenceNode {
    #[instrument(level = "debug", skip_all, name = "SequenceNode::execute", fields(
         nodes_count = %self.nodes.len(),
         path = ?path
     ))]
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext<'_>,
    ) -> BoxFuture<'a, ExecutionResult> {
        Box::pin(async move {
            let mut data = root.clone();
            let mut errors = vec![];
            let mut extensions = BTreeMap::new();
            for node in &self.nodes {
                let node_result = node.execute(&data, path.clone(), ctx).await;
                if let Some(node_data) = node_result.data {
                    data.deep_merge(node_data);
                }
                if let Some(node_errors) = node_result.errors {
                    errors.extend(node_errors);
                }
                if let Some(node_extensions) = node_result.extensions {
                    extensions.extend(node_extensions);
                }
            }
            ExecutionResult::new(Some(data), Some(errors), Some(extensions))
        })
    }
}
