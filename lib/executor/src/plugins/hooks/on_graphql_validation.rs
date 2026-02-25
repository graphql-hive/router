use std::sync::Arc;

use graphql_tools::{
    static_graphql::query::Document as QueryDocument,
    static_graphql::schema::Document as SchemaDocument,
    validation::{rules::ValidationRule, utils::ValidationError, validate::ValidationPlan},
};
use hive_router_query_planner::consumer_schema::ConsumerSchema;
use ntex::http::Response;

use crate::{
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{CacheHint, EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnGraphQLValidationStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub schema: Arc<ConsumerSchema>,
    pub document: Arc<QueryDocument>,
    pub validation_plan: Arc<ValidationPlan>,
    pub errors: Option<Arc<Vec<ValidationError>>>,
}

impl OnGraphQLValidationStartHookPayload<'_> {
    pub fn with_validation_plan<TValidationPlan: Into<ValidationPlan>>(
        mut self,
        validation_plan: TValidationPlan,
    ) -> Self {
        self.validation_plan = Arc::new(validation_plan.into());
        self
    }
    pub fn with_schema<TSchema: Into<Arc<SchemaDocument>>>(mut self, schema: TSchema) -> Self {
        let schema: Arc<SchemaDocument> = schema.into();
        let new_consumer_schema = ConsumerSchema::from(schema);
        self.schema = new_consumer_schema.into();
        self
    }
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
    pub fn with_validation_rule<TValidationRule: ValidationRule + 'static>(
        mut self,
        rule: TValidationRule,
    ) -> Self {
        let mut new_plan = self.validation_plan.as_ref().clone();
        new_plan.add_rule(Box::new(rule));
        self.validation_plan = Arc::new(new_plan);
        self
    }

    pub fn filter_validation_rules<F>(mut self, mut f: F) -> Self
    where
        F: FnMut(&Box<dyn ValidationRule>) -> bool,
    {
        let mut new_plan = self.validation_plan.as_ref().clone();
        new_plan.rules.retain(|rule| f(rule));
        self.validation_plan = Arc::new(new_plan);
        self
    }
}

pub struct OnGraphQLValidationEndHookPayload {
    pub errors: Arc<Vec<ValidationError>>,
    pub cache_hint: CacheHint,
}

impl EndHookPayload<Response> for OnGraphQLValidationEndHookPayload {}

pub type OnGraphQLValidationHookEndResult =
    EndHookResult<OnGraphQLValidationEndHookPayload, Response>;
