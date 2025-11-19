use graphql_tools::{
    static_graphql::query::Document,
    validation::{
        rules::{default_rules_validation_plan, ValidationRule},
        utils::ValidationError,
        validate::ValidationPlan,
    },
};
use hive_router_query_planner::state::supergraph_state::SchemaDocument;

use crate::plugin_trait::{EndPayload, StartPayload};

pub struct OnGraphQLValidationStartPayload<'exec> {
    pub router_http_request: &'exec mut ntex::web::HttpRequest,
    pub schema: &'exec SchemaDocument,
    pub document: &'exec Document,
    default_validation_plan: &'exec ValidationPlan,
    new_validation_plan: Option<ValidationPlan>,
    pub errors: Option<Vec<ValidationError>>,
}

impl<'exec> StartPayload<OnGraphQLValidationEndPayload> for OnGraphQLValidationStartPayload<'exec> {}

impl<'exec> OnGraphQLValidationStartPayload<'exec> {
    pub fn new(
        router_http_request: &'exec mut ntex::web::HttpRequest,
        schema: &'exec SchemaDocument,
        document: &'exec Document,
        default_validation_plan: &'exec ValidationPlan,
    ) -> Self {
        OnGraphQLValidationStartPayload {
            router_http_request,
            schema,
            document,
            default_validation_plan,
            new_validation_plan: None,
            errors: None,
        }
    }

    pub fn add_validation_rule(&mut self, rule: Box<dyn ValidationRule>) {
        self.new_validation_plan
            .get_or_insert_with(default_rules_validation_plan)
            .add_rule(rule);
    }

    pub fn filter_validation_rules<F>(&mut self, mut f: F)
    where
        F: FnMut(&Box<dyn ValidationRule>) -> bool,
    {
        let plan = self
            .new_validation_plan
            .get_or_insert_with(default_rules_validation_plan);
        plan.rules.retain(|rule| f(rule));
    }

    pub fn get_validation_plan(&self) -> &ValidationPlan {
        match &self.new_validation_plan {
            Some(plan) => plan,
            None => self.default_validation_plan,
        }
    }
}

pub struct OnGraphQLValidationEndPayload {
    pub errors: Vec<ValidationError>,
}

impl EndPayload for OnGraphQLValidationEndPayload {}
