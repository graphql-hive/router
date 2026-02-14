use std::sync::Arc;

use graphql_tools::{
    static_graphql::query::Document,
    validation::{
        rules::{default_rules_validation_plan, ValidationRule},
        utils::ValidationError,
        validate::ValidationPlan,
    },
};
use hive_router_query_planner::state::supergraph_state::SchemaDocument;
use ntex::http::Response;

use crate::{
    plugin_context::{PluginContext, PluginRequestState, RouterHttpRequest},
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnGraphQLValidationStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub schema: Arc<SchemaDocument>,
    pub document: Arc<Document>,
    default_validation_plan: &'exec ValidationPlan,
    // Override
    new_validation_plan: Option<ValidationPlan>,
    pub errors: Option<Vec<ValidationError>>,
}

impl<'exec> StartHookPayload<OnGraphQLValidationEndHookPayload, Response>
    for OnGraphQLValidationStartHookPayload<'exec>
{
}

pub type OnGraphQLValidationStartHookResult<'exec> = StartHookResult<
    'exec,
    OnGraphQLValidationStartHookPayload<'exec>,
    OnGraphQLValidationEndHookPayload,
    Response,
>;

impl<'exec> OnGraphQLValidationStartHookPayload<'exec> {
    pub fn new(
        plugin_req_state: &'exec PluginRequestState<'exec>,
        schema: Arc<SchemaDocument>,
        document: Arc<Document>,
        default_validation_plan: &'exec ValidationPlan,
    ) -> Self {
        OnGraphQLValidationStartHookPayload {
            router_http_request: &plugin_req_state.router_http_request,
            context: &plugin_req_state.context,
            schema,
            document,
            default_validation_plan,
            new_validation_plan: None,
            errors: None,
        }
    }

    pub fn with_validation_rule<TValidationRule: ValidationRule + 'static>(
        mut self,
        rule: TValidationRule,
    ) -> Self {
        self.new_validation_plan
            .get_or_insert_with(default_rules_validation_plan)
            .add_rule(Box::new(rule));
        self
    }

    pub fn filter_validation_rules<F>(mut self, mut f: F) -> Self
    where
        F: FnMut(&Box<dyn ValidationRule>) -> bool,
    {
        let plan = self
            .new_validation_plan
            .get_or_insert_with(default_rules_validation_plan);
        plan.rules.retain(|rule| f(rule));
        self
    }

    pub fn get_validation_plan(&self) -> &ValidationPlan {
        match &self.new_validation_plan {
            Some(plan) => plan,
            None => self.default_validation_plan,
        }
    }
}

pub struct OnGraphQLValidationEndHookPayload {
    pub errors: Vec<ValidationError>,
}

impl EndHookPayload<Response> for OnGraphQLValidationEndHookPayload {}

pub type OnGraphQLValidationHookEndResult =
    EndHookResult<OnGraphQLValidationEndHookPayload, Response>;
