use std::collections::BTreeMap;

use hive_router_config::introspection_policy::IntrospectionPermissionConfig;
use hive_router_internal::expressions::{
    values::boolean::BooleanOrProgram, CompileExpression, ExpressionCompileError,
};
use hive_router_plan_executor::{
    execution::client_request_details, introspection::resolve::IntrospectionContext,
    response::graphql_error::GraphQLError,
};
use vrl::core::Value as VrlValue;

use crate::pipeline::error::PipelineError;

pub fn compile_introspection_policy(
    introspection_policy_cfg: &Option<IntrospectionPermissionConfig>,
) -> Result<BooleanOrProgram, ExpressionCompileError> {
    match introspection_policy_cfg {
        Some(IntrospectionPermissionConfig::Boolean(b)) => Ok(BooleanOrProgram::Value(*b)),
        Some(IntrospectionPermissionConfig::Expression { expression }) => expression
            .compile_expression(None)
            .map(|program| BooleanOrProgram::Program(Box::new(program))),
        None => Ok(BooleanOrProgram::Value(true)),
    }
}

pub fn handle_introspection_policy(
    introspection_policy_prog: &BooleanOrProgram,
    introspection_context: &mut IntrospectionContext,
    client_request_details: &client_request_details::ClientRequestDetails<'_>,
    initial_errors: &mut Vec<GraphQLError>,
) -> Result<(), PipelineError> {
    let is_enabled = introspection_policy_prog
        .resolve(|| {
            let mut context_map = BTreeMap::new();
            context_map.insert("request".into(), client_request_details.into());

            VrlValue::Object(context_map)
        })
        .map_err(|e| PipelineError::IntrospectionPermissionEvaluationError(e.to_string()))?;

    if !is_enabled {
        introspection_context.query = None;
        initial_errors.push(create_introspection_disabled_error());
    }

    Ok(())
}

pub fn create_introspection_disabled_error() -> GraphQLError {
    GraphQLError::from_message_and_code(
        "Introspection queries are disabled.",
        "INTROSPECTION_DISABLED",
    )
}
