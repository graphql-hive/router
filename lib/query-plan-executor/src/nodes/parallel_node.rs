use std::collections::BTreeMap;

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use query_planner::planner::plan_nodes::ParallelNode;
use serde_json::Value;
use tracing::instrument;

use crate::deep_merge::DeepMerge;
use crate::execution_context::ExecutionContext;
use crate::execution_result::ExecutionResult;
use crate::nodes::plan_node::ExecutablePlanNode;

pub trait ExecutableParallelNode {
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext,
    ) -> BoxFuture<'a, ExecutionResult>;
}

impl ExecutableParallelNode for ParallelNode {
    #[instrument(level = "debug", skip_all, name = "ParallelNode::execute", fields(
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
            let mut stream: FuturesUnordered<_> = self
                .nodes
                .iter()
                .map(|node| node.execute(root, path.clone(), ctx))
                .collect();
            let mut data = Value::Null;
            let mut errors = vec![];
            let mut extensions = BTreeMap::new();
            while let Some(node_result) = stream.next().await {
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
