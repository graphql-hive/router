use hive_router_config::introspection_policy::IntrospectionPermissionConfig;
use hive_router_internal::expressions::{
    values::boolean::BooleanOrProgram, CompileExpression, ExpressionCompileError, VrlView,
};
use hive_router_plan_executor::execution::client_request_details::{self, ClientRequestView};
use tracing::debug;

use crate::pipeline::error::PipelineError;

pub fn compile_introspection_policy(
    introspection_policy_cfg: &Option<IntrospectionPermissionConfig>,
) -> Result<BooleanOrProgram, ExpressionCompileError> {
    match introspection_policy_cfg {
        Some(IntrospectionPermissionConfig::Boolean(b)) => Ok(BooleanOrProgram::Value(*b)),
        Some(IntrospectionPermissionConfig::Expression { expression }) => {
            expression.compile_expression(None).map(|program| {
                let hints = hive_router_internal::expressions::ProgramHints::from_program(&program);
                BooleanOrProgram::Program(Box::new(program), hints)
            })
        }
        None => Ok(BooleanOrProgram::Value(true)),
    }
}

pub fn handle_introspection_policy(
    introspection_policy_prog: &BooleanOrProgram,
    client_request_details: &client_request_details::ClientRequestDetails<'_>,
) -> Result<(), PipelineError> {
    let is_enabled = introspection_policy_prog
        .resolve_with_hints(|hints| {
            hints.context_builder(|root| {
                root.insert_object("request", |req| {
                    ClientRequestView::new(client_request_details).write(req);
                });
            })
        })
        .map_err(|e| PipelineError::IntrospectionPermissionEvaluationError(e.to_string()))?;

    if !is_enabled {
        debug!("graphql request rejected because introspection is disabled");
        Err(PipelineError::IntrospectionDisabled)
    } else {
        Ok(())
    }
}
