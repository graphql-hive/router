use hive_router::http::StatusCode;
use hive_router::query_planner::ast::selection_item::SelectionItem;
use hive_router::{
    async_trait,
    graphql_tools::{
        ast::{visit_document, OperationVisitor, OperationVisitorContext},
        static_graphql,
        validation::{
            rules::ValidationRule,
            utils::{ValidationError, ValidationErrorContext},
        },
    },
    tracing, GraphQLError,
};
use serde::Deserialize;

use hive_router::plugins::{
    hooks::{
        on_graphql_validation::{
            OnGraphQLValidationStartHookPayload, OnGraphQLValidationStartHookResult,
        },
        on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
        on_query_plan::{OnQueryPlanStartHookPayload, OnQueryPlanStartHookResult},
    },
    plugin_trait::{RouterPlugin, StartHookPayload},
};

// This example shows two ways of limiting the number of root fields in a query:
// 1. During validation step
// 2. During query planning step

#[async_trait]
impl RouterPlugin for RootFieldLimitPlugin {
    type Config = RootFieldLimitPluginConfig;
    fn plugin_name() -> &'static str {
        "root_field_limit"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        payload.initialize_plugin(Self {
            max_root_fields: payload.config()?.max_root_fields,
        })
    }
    // Using validation step
    async fn on_graphql_validation<'exec>(
        &'exec self,
        payload: OnGraphQLValidationStartHookPayload<'exec>,
    ) -> OnGraphQLValidationStartHookResult<'exec> {
        let rule = RootFieldLimitRule {
            max_root_fields: self.max_root_fields,
        };

        payload.with_validation_rule(rule).proceed()
    }
    // Or during query planning
    async fn on_query_plan<'exec>(
        &'exec self,
        payload: OnQueryPlanStartHookPayload<'exec>,
    ) -> OnQueryPlanStartHookResult<'exec> {
        let mut cnt = 0;
        for selection in payload
            .filtered_operation_for_plan
            .selection_set
            .items
            .iter()
        {
            match selection {
                SelectionItem::Field(_) => {
                    cnt += 1;
                    if cnt > self.max_root_fields {
                        let err_msg = format!(
                            "Query has too many root fields: {}, maximum allowed is {}",
                            cnt, self.max_root_fields
                        );
                        tracing::warn!("{}", err_msg);
                        // Return error
                        return payload.end_with_graphql_error(
                            GraphQLError::from_message_and_code(err_msg, "TOO_MANY_ROOT_FIELD"),
                            StatusCode::PAYLOAD_TOO_LARGE,
                        );
                    }
                }
                SelectionItem::InlineFragment(_) => {
                    unreachable!("Inline fragments should have been inlined before query planning");
                }
                SelectionItem::FragmentSpread(_) => {
                    unreachable!("Fragment spreads should have been inlined before query planning");
                }
            }
        }
        payload.proceed()
    }
}

#[derive(Deserialize)]
pub struct RootFieldLimitPluginConfig {
    max_root_fields: usize,
}

pub struct RootFieldLimitPlugin {
    max_root_fields: usize,
}

pub struct RootFieldLimitRule {
    max_root_fields: usize,
}

struct RootFieldSelections {
    max_root_fields: usize,
    count: usize,
}

impl<'a> OperationVisitor<'a, ValidationErrorContext> for RootFieldSelections {
    fn enter_field(
        &mut self,
        visitor_context: &mut OperationVisitorContext,
        user_context: &mut ValidationErrorContext,
        field: &static_graphql::query::Field,
    ) {
        let parent_type_name = visitor_context.current_parent_type().map(|t| t.name());
        if parent_type_name == Some("Query") {
            self.count += 1;
            if self.count > self.max_root_fields {
                let err_msg = format!(
                    "Query has too many root fields: {}, maximum allowed is {}",
                    self.count, self.max_root_fields
                );
                user_context.report_error(ValidationError {
                    error_code: "TOO_MANY_ROOT_FIELDS",
                    locations: vec![field.position],
                    message: err_msg,
                });
            }
        }
    }
}

impl ValidationRule for RootFieldLimitRule {
    fn error_code<'a>(&self) -> &'a str {
        "TOO_MANY_ROOT_FIELDS"
    }
    fn validate(
        &self,
        ctx: &mut OperationVisitorContext<'_>,
        error_collector: &mut ValidationErrorContext,
    ) {
        visit_document(
            &mut RootFieldSelections {
                max_root_fields: self.max_root_fields,
                count: 0,
            },
            ctx.operation,
            ctx,
            error_collector,
        );
    }
}

#[cfg(test)]
mod tests {
    use e2e::testkit::{
        init_router_from_config_file_with_plugins, wait_for_readiness, SubgraphsServer,
    };
    use hive_router::{
        ntex,
        sonic_rs::{self, JsonValueTrait},
        PluginRegistry,
    };
    use ntex::web::test;
    #[ntex::test]
    async fn rejects_query_with_too_many_root_fields() {
        SubgraphsServer::start().await;
        let app = init_router_from_config_file_with_plugins(
            "../plugin_examples/root_field_limit/router.config.yaml",
            PluginRegistry::new().register::<super::RootFieldLimitPlugin>(),
        )
        .await
        .expect("failed to start router");
        wait_for_readiness(&app.app).await;
        let resp = test::call_service(
            &app.app,
            test::TestRequest::post()
                .uri("/graphql")
                .set_payload(
                    r#"{"query":"query TooManyRootFields { users { id } topProducts { upc } }"}"#,
                )
                .header("content-type", "application/json")
                .to_request(),
        )
        .await;
        let json_body: sonic_rs::Value =
            sonic_rs::from_slice(&test::read_body(resp).await).unwrap();

        let error_msg = json_body["errors"][0]["message"].as_str().unwrap();
        assert!(
            error_msg.contains("Query has too many root fields"),
            "Unexpected error message: {}",
            error_msg
        );
    }
}
