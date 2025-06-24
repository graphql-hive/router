use futures::future::BoxFuture;
use query_planner::planner::plan_nodes::SequenceNode;
use serde_json::{Map, Value};

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
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext<'_>,
    ) -> BoxFuture<'a, ExecutionResult> {
        Box::pin(async move {
            let mut data = root.clone();
            let mut errors = vec![];
            let mut extensions = Map::new();
            for node in &self.nodes {
                let node_result = node.execute(&data, path.clone(), ctx).await;
                if let Some(node_data) = node_result.data {
                    data.deep_merge(node_data);
                }
                if let Some(node_errors) = node_result.errors {
                    errors.extend(node_errors);
                }
                if let Some(node_extensions) = node_result.extensions {
                    extensions.deep_merge(node_extensions);
                }
            }
            ExecutionResult::new(Some(data), Some(errors), Some(extensions))
        })
    }
}
