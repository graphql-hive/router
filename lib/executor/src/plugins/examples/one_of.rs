// This example will show `@oneOf` input type validation in two steps:
// 1. During validation step
// 2. During execution step

// We handle execution too to validate input objects at runtime as well.

use std::{collections::BTreeMap, sync::RwLock};

use crate::{
    hooks::{
        on_execute::{OnExecuteEndPayload, OnExecuteStartPayload},
        on_graphql_validation::{OnGraphQLValidationEndPayload, OnGraphQLValidationStartPayload},
        on_supergraph_load::{OnSupergraphLoadEndPayload, OnSupergraphLoadStartPayload},
    },
    plugin_trait::{HookResult, RouterPlugin, StartPayload},
};
use graphql_parser::{
    query::Value,
    schema::{Definition, TypeDefinition},
};
use graphql_tools::ast::visit_document;
use graphql_tools::{
    ast::{OperationVisitor, OperationVisitorContext},
    validation::{
        rules::ValidationRule,
        utils::{ValidationError, ValidationErrorContext},
    },
};

pub struct OneOfPlugin {
    pub one_of_types: RwLock<Vec<String>>,
}

#[async_trait::async_trait]
impl RouterPlugin for OneOfPlugin {
    // 1. During validation step
    async fn on_graphql_validation<'exec>(
        &'exec self,
        mut payload: OnGraphQLValidationStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLValidationStartPayload<'exec>, OnGraphQLValidationEndPayload>
    {
        let rule = OneOfValidationRule {
            one_of_types: self.one_of_types.read().unwrap().clone(),
        };
        payload.add_validation_rule(Box::new(rule));
        payload.cont()
    }
    // 2. During execution step
    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartPayload<'exec>,
    ) -> HookResult<'exec, OnExecuteStartPayload<'exec>, OnExecuteEndPayload> {
        payload.cont()
    }
    fn on_supergraph_reload<'exec>(
        &'exec self,
        start_payload: OnSupergraphLoadStartPayload,
    ) -> HookResult<'exec, OnSupergraphLoadStartPayload, OnSupergraphLoadEndPayload> {
        for def in start_payload.new_ast.definitions.iter() {
            if let Definition::TypeDefinition(TypeDefinition::InputObject(input_obj)) = def {
                for directive in input_obj.directives.iter() {
                    if directive.name == "oneOf" {
                        self.one_of_types
                            .write()
                            .unwrap()
                            .push(input_obj.name.clone());
                    }
                }
            }
        }
        start_payload.cont()
    }
}

struct OneOfValidationRule {
    one_of_types: Vec<String>,
}

impl ValidationRule for OneOfValidationRule {
    fn error_code<'a>(&self) -> &'a str {
        "TOO_MANY_ROOT_FIELDS"
    }
    fn validate(
        &self,
        op_ctx: &mut OperationVisitorContext<'_>,
        validation_error_context: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut OneOfValidation {
                one_of_types: self.one_of_types.clone(),
            },
            op_ctx.operation,
            op_ctx,
            validation_error_context,
        );
    }
}

struct OneOfValidation {
    one_of_types: Vec<String>,
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for OneOfValidation {
    fn enter_object_value(
        &mut self,
        visitor_context: &mut OperationVisitorContext<'a>,
        user_context: &mut ValidationErrorContext,
        fields: &BTreeMap<String, graphql_tools::static_graphql::query::Value>,
    ) {
        if let Some(TypeDefinition::InputObject(input_type)) = visitor_context.current_input_type()
        {
            if self.one_of_types.contains(&input_type.name) {
                let mut set_fields = vec![];
                for (field_name, field_value) in fields.iter() {
                    if !matches!(field_value, Value::Null) {
                        set_fields.push(field_name.clone());
                    }
                }
                if set_fields.len() > 1 {
                    let err_msg = format!(
                        "Input object of type '{}' with @oneOf directive has multiple fields set: {:?}. Only one field must be set.",
                        input_type.name,
                        set_fields
                    );
                    user_context.report_error(ValidationError {
                        error_code: "TOO_MANY_FIELDS_SET_IN_ONEOF",
                        locations: vec![],
                        message: err_msg,
                    });
                }
            }
        }
    }
}
