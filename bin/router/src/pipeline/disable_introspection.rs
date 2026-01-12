use std::collections::BTreeMap;

use hive_router_config::disable_introspection::DisableIntrospectionConfig;
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

pub fn compile_disable_introspection(
    disable_introspection_config: &Option<DisableIntrospectionConfig>,
) -> Result<Option<BooleanOrProgram>, ExpressionCompileError> {
    disable_introspection_config
        .as_ref()
        .map(|config| match config {
            DisableIntrospectionConfig::Boolean(b) => Ok(BooleanOrProgram::Value(*b)),
            DisableIntrospectionConfig::Expression { expression } => expression
                .compile_expression(None)
                .map(|program| BooleanOrProgram::Program(Box::new(program))),
        })
        .transpose()
}

pub fn handle_disable_introspection(
    disable_introspection_program: &BooleanOrProgram,
    introspection_context: &mut IntrospectionContext,
    client_request_details: &client_request_details::ClientRequestDetails<'_, '_>,
    default_value: bool,
    initial_errors: &mut Vec<GraphQLError>,
) -> Result<(), PipelineError> {
    let is_disabled = disable_introspection_program.resolve(|| {
        let mut context_map = BTreeMap::new();
        context_map.insert("request".into(), client_request_details.into());
        VrlValue::Object(context_map)
    });

    match is_disabled {
        Ok(true) => {
            initial_errors.push(create_disable_introspection_error());
            introspection_context.query = None;
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(e) => Err(PipelineError::DisableIntrospectionEvaluationError(
            e.to_string(),
        )),
    }
}

pub fn create_disable_introspection_error() -> GraphQLError {
    GraphQLError::from_message_and_extensions(
        "Introspection queries are disabled.".into(),
        GraphQLErrorExtensions::new_from_code("INTROSPECTION_DISABLED"),
    )
}
