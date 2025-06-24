use futures::future::BoxFuture;
use query_planner::planner::plan_nodes::{ConditionNode, PlanNode};
use serde_json::Value;

use crate::{
    execution_context::ExecutionContext, execution_result::ExecutionResult,
    nodes::plan_node::ExecutablePlanNode,
};

pub trait ExecutableConditionNode {
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext,
    ) -> BoxFuture<'a, ExecutionResult>;
    fn inner_node(&self, ctx: &ExecutionContext) -> Option<&Box<PlanNode>>;
}

impl ExecutableConditionNode for ConditionNode {
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext,
    ) -> BoxFuture<'a, ExecutionResult> {
        // Get the inner node based on the condition
        let inner_node = self.inner_node(ctx);
        if let Some(node) = inner_node {
            // Execute the inner node with the provided root, path, and context
            node.execute(root, path, ctx)
        } else {
            // If no inner node is found, return an empty ExecutionResult
            Box::pin(async move { ExecutionResult::default() })
        }
    }
    fn inner_node(&self, ctx: &ExecutionContext) -> Option<&Box<PlanNode>> {
        // Get the condition variable from the context
        let condition_value: bool = match ctx.variables {
            Some(ref variable_values) => {
                match variable_values.get(&self.condition) {
                    Some(value) => {
                        // Check if the value is a boolean
                        match value {
                            Value::Bool(b) => *b,
                            _ => true, // Default to true if not a boolean
                        }
                    }
                    None => {
                        // If the variable is not found, default to false
                        false
                    }
                }
            }
            None => {
                // No variable values provided, default to false
                false
            }
        };
        if condition_value {
            self.if_clause.as_ref()
        } else {
            self.else_clause.as_ref()
        }
    }
}
