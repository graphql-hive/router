use std::collections::BTreeMap;

use hive_router_config::allow_introspection::AllowIntrospectionConfig;
use hive_router_internal::expressions::{
    values::boolean::BooleanOrProgram, CompileExpression, ExpressionCompileError,
};
use hive_router_plan_executor::{
    execution::client_request_details,
    introspection::resolve::IntrospectionContext,
    response::graphql_error::{GraphQLError, GraphQLErrorExtensions},
};
use vrl::core::Value as VrlValue;

use crate::pipeline::error::{PipelineError};

pub fn compile_allow_introspection(
    allow_introspection_config: &Option<AllowIntrospectionConfig>,
) -> Result<Option<BooleanOrProgram>, ExpressionCompileError> {
    allow_introspection_config
        .as_ref()
        .map(|config| match config {
            AllowIntrospectionConfig::Boolean(b) => Ok(BooleanOrProgram::Value(*b)),
            AllowIntrospectionConfig::Expression { expression } => expression
                .compile_expression(None)
                .map(|program| BooleanOrProgram::Program(Box::new(program))),
        })
        .transpose()
}

pub fn handle_allow_introspection(
    allow_introspection_program: &BooleanOrProgram,
    introspection_context: &mut IntrospectionContext,
    client_request_details: &client_request_details::ClientRequestDetails<'_, '_>,
    default_value: bool,
    initial_errors: &mut Vec<GraphQLError>,
) -> Result<(), PipelineError> {
    let is_enabled = allow_introspection_program
        .resolve(|| {
            let mut context_map = BTreeMap::new();
            context_map.insert("request".into(), client_request_details.into());

            context_map.insert("default".into(), VrlValue::Boolean(default_value));

            VrlValue::Object(context_map)
        })
        .map_err(|e| PipelineError::AllowIntrospectionEvaluationError(e.to_string()))?;

    if !is_enabled {
        introspection_context.query = None;
        initial_errors.push(create_introspection_disabled_error());
    }

    Ok(())
}

pub fn create_introspection_disabled_error() -> GraphQLError {
    GraphQLError::from_message_and_extensions(
        "Introspection queries are disabled.".into(),
        GraphQLErrorExtensions::new_from_code("INTROSPECTION_DISABLED"),
    )
}
