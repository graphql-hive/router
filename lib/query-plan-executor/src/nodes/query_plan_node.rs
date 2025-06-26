use std::collections::BTreeMap;

use futures::future::BoxFuture;
use query_planner::{ast::operation::OperationDefinition, planner::plan_nodes::QueryPlan};
use serde_json::{Map, Value};

use crate::{
    execution_context::{self, ExecutionContext},
    execution_result::ExecutionResult,
    executors::{self, map::SubgraphExecutorMap},
    nodes::plan_node::ExecutablePlanNode,
    projection,
    schema_metadata::SchemaMetadata,
};

pub trait ExecutableQueryPlanNode {
    fn execute<'a>(&'a self, ctx: &'a ExecutionContext<'_>) -> BoxFuture<'a, ExecutionResult>;
    fn execute_operation<'a>(
        &'a self,
        subgraph_executor_map: &'a SubgraphExecutorMap,
        variables: &'a Option<BTreeMap<String, Value>>,
        schema_metadata: &'a SchemaMetadata,
        operation: &'a OperationDefinition,
        has_introspection: bool,
        expose_query_plan: bool,
    ) -> BoxFuture<'a, String>;
}

impl ExecutableQueryPlanNode for QueryPlan {
    fn execute<'a>(&'a self, ctx: &'a ExecutionContext<'_>) -> BoxFuture<'a, ExecutionResult> {
        if let Some(node) = &self.node {
            node.execute(&Value::Null, vec![], ctx)
        } else {
            Box::pin(async move { ExecutionResult::default() })
        }
    }

    fn execute_operation<'a>(
        &'a self,
        subgraph_executor_map: &'a executors::map::SubgraphExecutorMap,
        variables: &'a Option<BTreeMap<String, Value>>,
        schema_metadata: &'a SchemaMetadata,
        operation: &'a OperationDefinition,
        has_introspection: bool,
        expose_query_plan: bool,
    ) -> BoxFuture<'a, String> {
        Box::pin(async move {
            let ctx = execution_context::ExecutionContext {
                subgraph_executor_map,
                schema_metadata,
                variables,
            };
            let mut result = self.execute(&ctx).await;
            if result.data.is_none() && has_introspection {
                result.data = Some(Value::Object(Map::new()));
            }
            if expose_query_plan {
                let extensions = result.extensions.get_or_insert_with(BTreeMap::new);
                extensions.insert(
                    "queryPlan".to_string(),
                    serde_json::to_value(self).expect("Failed to serialize query plan"),
                );
            }
            projection::project_by_operation(result, operation, schema_metadata, variables)
        })
    }
}
