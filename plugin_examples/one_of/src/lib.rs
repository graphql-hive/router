// This example will show `@oneOf` input type validation in two steps:
// 1. During validation step
// 2. During execution step

// We handle execution too to validate input objects at runtime as well.
/*
    Let's say we have the following input type with `@oneOf` directive:
    input PaymentMethod @oneOf {
        creditCard: CreditCardInput
        bankTransfer: BankTransferInput
        paypal: PayPalInput
    }

    During validation, if a variable of type `PaymentMethod` is provided with multiple fields set,
    we will raise a validation error.

    ```graphql
    mutation MakePayment {
        makePayment(method: {
            creditCard: { number: "1234", expiry: "12/24" },
            paypal: { email: "john@doe.com" }
        }) {
            success
        }
    }
    ```

    But since variables can be dynamic, we also validate during execution. If the input object has multiple fields set,
    we return an error in the response.

    ```graphql
    mutation MakePayment($method: PaymentMethod!) {
        makePayment(method: $method) {
            success
        }
    }
    ```

    with variables:
    {
        "method": {
            "creditCard": { "number": "1234", "expiry": "12/24" },
            "paypal": { "email": "john@doe.com" }
        }
    }
*/

use std::{collections::BTreeMap, sync::Arc};

use hive_router::graphql_tools::ast::{visit_document, OperationVisitor, OperationVisitorContext};
use hive_router::graphql_tools::parser::schema::Definition;
use hive_router::graphql_tools::static_graphql::schema::{TypeDefinition, Value};
use hive_router::graphql_tools::validation::rules::ValidationRule;
use hive_router::graphql_tools::validation::utils::{ValidationError, ValidationErrorContext};
use hive_router::plugins::hooks::on_plugin_init::{OnPluginInitPayload, OnPluginInitResult};
use hive_router::plugins::{
    hooks::{
        on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult},
        on_graphql_validation::{
            OnGraphQLValidationStartHookPayload, OnGraphQLValidationStartHookResult,
        },
        on_supergraph_load::{OnSupergraphLoadStartHookPayload, OnSupergraphLoadStartHookResult},
    },
    plugin_trait::{RouterPlugin, StartHookPayload},
};
use hive_router::sonic_rs::JsonContainerTrait;
use hive_router::{async_trait, graphql_tools, http, ArcSwap, GraphQLError};

#[derive(Default)]
pub struct OneOfPlugin {
    pub one_of_types: ArcSwap<Vec<String>>,
}

#[async_trait]
impl RouterPlugin for OneOfPlugin {
    type Config = ();
    fn plugin_name() -> &'static str {
        "oneof"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin_with_defaults()
    }
    // 1. During validation step
    async fn on_graphql_validation<'exec>(
        &'exec self,
        payload: OnGraphQLValidationStartHookPayload<'exec>,
    ) -> OnGraphQLValidationStartHookResult<'exec> {
        let one_of_types = self.one_of_types.load();
        // If there are no oneOf types, skip adding the validation rule
        if one_of_types.is_empty() {
            return payload.proceed();
        }
        let rule = OneOfValidationRule {
            one_of_types: one_of_types.clone(),
        };
        payload.with_validation_rule(rule).proceed()
    }
    // 2. During execution step
    async fn on_execute<'exec>(
        &'exec self,
        payload: OnExecuteStartHookPayload<'exec>,
    ) -> OnExecuteStartHookResult<'exec> {
        let one_of_types = self.one_of_types.load();
        // If there are no oneOf types, skip checking
        if one_of_types.is_empty() {
            return payload.proceed();
        }
        if let (Some(variable_values), Some(variable_defs)) = (
            &payload.variable_values,
            &payload.operation_for_plan.variable_definitions,
        ) {
            for def in variable_defs {
                let variable_named_type = def.variable_type.inner_type();
                if one_of_types.contains(&variable_named_type.to_string()) {
                    let var_name = &def.name;
                    if let Some(value) = variable_values.get(var_name).and_then(|v| v.as_object()) {
                        let keys_num = value.len();
                        if keys_num > 1 {
                            return payload.end_with_graphql_error(GraphQLError::from_message_and_code(
                                format!(
                                    "Variable '${}' of input object type '{}' with @oneOf directive has multiple fields set: {:?}. Only one field must be set.",
                                    var_name,
                                    variable_named_type,
                                    keys_num
                                ),
                                "TOO_MANY_FIELDS_SET_IN_ONEOF",
                            ),
                            http::StatusCode::PAYLOAD_TOO_LARGE
                        );
                        }
                    }
                }
            }
        }
        payload.proceed()
    }
    fn on_supergraph_reload<'exec>(
        &'exec self,
        start_payload: OnSupergraphLoadStartHookPayload,
    ) -> OnSupergraphLoadStartHookResult<'exec> {
        let mut one_of_types = vec![];
        for def in start_payload.new_ast.definitions.iter() {
            if let Definition::TypeDefinition(TypeDefinition::InputObject(input_obj)) = def {
                for directive in input_obj.directives.iter() {
                    if directive.name == "oneOf" {
                        one_of_types.push(input_obj.name.clone());
                    }
                }
            }
        }
        self.one_of_types.store(one_of_types.into());
        start_payload.proceed()
    }
}

struct OneOfValidationRule {
    one_of_types: Arc<Vec<String>>,
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
    one_of_types: Arc<Vec<String>>,
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

#[cfg(test)]
mod tests {
    use hive_router::sonic_rs::{from_slice, JsonContainerTrait, JsonValueTrait, Value};
    use hive_router::{ntex, sonic_rs, PluginRegistry};

    use e2e::testkit::{init_router_from_config_file_with_plugins, wait_for_readiness};

    #[ntex::test]
    async fn one_of_validates_in_validation_rule() {
        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/one_of/router.config.yaml",
            PluginRegistry::new().register::<super::OneOfPlugin>(),
        )
        .await
        .expect("Router should initialize successfully");
        wait_for_readiness(&app.app).await;

        let req = e2e::testkit::init_graphql_request(
            r#"
            mutation OneOfTest {
                oneofTest(input: {
                    string: "test",
                    int: 42
            }) {
                    string
                    int
                    float
                    boolean
                    id
                }
            }
            "#,
            None,
        );

        let resp = ntex::web::test::call_service(&app.app, req.to_request()).await;
        let body = ntex::web::test::read_body(resp).await;
        let body_val: Value = from_slice(&body).expect("Response body should be valid JSON");
        let errors = body_val
            .get("errors")
            .expect("Response should contain errors");
        let first_error = errors
            .as_array()
            .expect("Errors should be an array")
            .first()
            .expect("There should be at least one error");
        let message = first_error
            .get("message")
            .expect("Error should have a message")
            .as_str()
            .expect("Message should be a string");
        assert!(message.contains("multiple fields set"));
    }

    #[ntex::test]
    async fn one_of_validates_during_execution() {
        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/one_of/router.config.yaml",
            PluginRegistry::new().register::<super::OneOfPlugin>(),
        )
        .await
        .expect("Router should initialize successfully");
        wait_for_readiness(&app.app).await;

        let req = e2e::testkit::init_graphql_request(
            r#"
            mutation OneOfTest($input: OneOfTestInput!) {
                oneofTest(input: $input) {
                    string
                    int
                    float
                    boolean
                    id
                }
            }
            "#,
            Some(sonic_rs::json!({
                "input": {
                    "string": "test",
                    "int": 42
                }
            })),
        );

        let resp = ntex::web::test::call_service(&app.app, req.to_request()).await;
        let body = ntex::web::test::read_body(resp).await;
        let body_val: Value = from_slice(&body).expect("Response body should be valid JSON");
        let errors = body_val
            .get("errors")
            .expect("Response should contain errors");
        let first_error = errors
            .as_array()
            .expect("Errors should be an array")
            .first()
            .expect("There should be at least one error");
        let message = first_error
            .get("message")
            .expect("Error should have a message")
            .as_str()
            .expect("Message should be a string");
        assert!(message.contains("multiple fields set"));
    }
}
