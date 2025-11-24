use graphql_tools::{
    ast::{visit_document, OperationVisitor, OperationVisitorContext, TypeDefinitionExtension},
    static_graphql,
    validation::{
        rules::ValidationRule,
        utils::{ValidationError, ValidationErrorContext},
    },
};
use hive_router_query_planner::ast::selection_item::SelectionItem;
use serde::Deserialize;
use sonic_rs::json;

use crate::{
    execution::plan::PlanExecutionOutput,
    hooks::{
        on_graphql_validation::{OnGraphQLValidationEndPayload, OnGraphQLValidationStartPayload},
        on_query_plan::{OnQueryPlanEndPayload, OnQueryPlanStartPayload},
    },
    plugin_trait::{HookResult, RouterPlugin, RouterPluginWithConfig, StartPayload},
};

// This example shows two ways of limiting the number of root fields in a query:
// 1. During validation step
// 2. During query planning step

#[async_trait::async_trait]
impl RouterPlugin for RootFieldLimitPlugin {
    // Using validation step
    async fn on_graphql_validation<'exec>(
        &'exec self,
        mut payload: OnGraphQLValidationStartPayload<'exec>,
    ) -> HookResult<'exec, OnGraphQLValidationStartPayload<'exec>, OnGraphQLValidationEndPayload>
    {
        let rule = RootFieldLimitRule {
            max_root_fields: self.max_root_fields,
        };
        payload.add_validation_rule(Box::new(rule));
        payload.cont()
    }
    // Or during query planning
    async fn on_query_plan<'exec>(
        &'exec self,
        payload: OnQueryPlanStartPayload<'exec>,
    ) -> HookResult<'exec, OnQueryPlanStartPayload<'exec>, OnQueryPlanEndPayload> {
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
                        let body = json!({
                            "errors": [{
                                "message": err_msg,
                                "extensions": {
                                    "code": "TOO_MANY_ROOT_FIELDS"
                                }
                            }]
                        });
                        // Return error
                        return payload.end_response(PlanExecutionOutput {
                            body: sonic_rs::to_vec(&body).unwrap_or_default(),
                            headers: http::HeaderMap::new(),
                            status: http::StatusCode::PAYLOAD_TOO_LARGE,
                        });
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
        payload.cont()
    }
}

#[derive(Deserialize)]
pub struct RootFieldLimitPluginConfig {
    enabled: bool,
    max_root_fields: usize,
}

impl RouterPluginWithConfig for RootFieldLimitPlugin {
    type Config = RootFieldLimitPluginConfig;
    fn plugin_name() -> &'static str {
        "root_field_limit_plugin"
    }
    fn from_config(config: Self::Config) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        Some(RootFieldLimitPlugin {
            max_root_fields: config.max_root_fields,
        })
    }
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
