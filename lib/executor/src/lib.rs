use std::collections::HashMap;

use bytes::BytesMut;
use query_planner::planner::plan_nodes::QueryPlan;

use crate::{
    context::ExecutionContext,
    projection::{plan::ProjectionPlan, response::project_by_operation},
    response::value::Value,
    schema::metadata::SchemaMetadata,
};

mod consts;
pub mod context;
mod json_writer;
pub mod projection;
pub mod response;
pub mod schema;

pub async fn execute_query_plan<'a>(
    query_plan: &QueryPlan,
    projection_plan: &ProjectionPlan<'a>,
    // subgraph_executor_map: &SubgraphExecutorMap,
    variable_values: &Option<HashMap<String, serde_json::Value>>,
    schema_metadata: &SchemaMetadata<'a>,
    operation_type_name: &str,
    // selections: &Vec<FieldProjectionPlan>,
    has_introspection: bool,
    // expose_query_plan: ExposeQueryPlanMode,
) -> BytesMut {
    let mut result_data = if has_introspection {
        // schema_metadata.introspection_query_json.clone()
        Value::Null
    } else {
        Value::Null
    };

    let execution_context = ExecutionContext::new(schema_metadata);

    // let mut result_errors = vec![]; // Initial errors are empty
    // let mut result_extensions = if expose_query_plan == ExposeQueryPlanMode::Yes
    //     || expose_query_plan == ExposeQueryPlanMode::DryRun
    // {
    //     HashMap::from_iter([(
    //         "queryPlan".to_string(),
    //         serde_json::to_value(query_plan).unwrap(),
    //     )])
    // } else {
    //     HashMap::new()
    // };
    // let mut execution_context = QueryPlanExecutionContext {
    //     variable_values,
    //     subgraph_executor_map,
    //     schema_metadata,
    //     errors: result_errors,
    //     extensions: result_extensions,
    //     response_storage: ResponsesStorage::new(),
    // };
    // if expose_query_plan != ExposeQueryPlanMode::DryRun {
    //     query_plan.execute(&mut execution_context).await;
    // }
    // result_errors = execution_context.errors; // Get the final errors from the execution context
    // result_extensions = execution_context.extensions; // Get the final extensions from the execution context
    project_by_operation(
        &execution_context.response_storage.final_response,
        // &mut result_errors,
        // &result_extensions,
        operation_type_name,
        &projection_plan.root_selections,
        variable_values,
    )
}
