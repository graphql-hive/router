use query_planner::ast::operation::OperationDefinition;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
// For reading file in main

use crate::{nodes::query_plan_node::ExecutableQueryPlanNode, schema_metadata::SchemaMetadata};
pub mod deep_merge;
pub mod execution_context;
pub mod execution_result;
pub mod executors;
pub mod introspection;
pub mod nodes;
pub mod projection;
pub mod schema_metadata;
pub mod traverse_path;
pub mod validation;
mod value_from_ast;
pub mod variables;

const TYPENAME_FIELD: &str = "__typename";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GraphQLError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<GraphQLErrorLocation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<Value>>, // Path can be string or number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, Value>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GraphQLErrorLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Map<String, Value>>,
}

// --- Main Function (for testing) ---

pub async fn execute_query_plan(
    query_plan: &query_planner::planner::plan_nodes::QueryPlan,
    subgraph_executor_map: &executors::map::SubgraphExecutorMap,
    variables: &Option<Map<String, Value>>,
    schema_metadata: &SchemaMetadata,
    operation: &OperationDefinition,
    has_introspection: bool,
) -> execution_result::ExecutionResult {
    let ctx = execution_context::ExecutionContext {
        subgraph_executor_map,
        schema_metadata,
        variables,
    };
    let mut result = query_plan.execute(&ctx).await;
    if result.data.is_none() && has_introspection {
        result.data = Some(Value::Object(Map::new()));
    }
    if let Some(ref mut data) = result.data {
        let mut errors = result.errors.take().unwrap_or_default();
        projection::project_data_by_operation(
            data,
            &mut errors,
            operation,
            schema_metadata,
            ctx.variables,
        );
        result.errors = if errors.is_empty() {
            None
        } else {
            Some(errors)
        };
    }
    result
}

#[cfg(test)]
mod tests;
