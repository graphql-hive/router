use graphql_tools::{static_graphql::query::Document, validation::{utils::ValidationError, validate::ValidationPlan}};
use hive_router_query_planner::state::supergraph_state::SchemaDocument;

use crate::{hooks::on_deserialization::GraphQLParams, plugin_trait::{EndPayload, StartPayload}};

pub struct OnGraphQLValidationStartPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub graphql_params: &'exec GraphQLParams,
    pub schema: &'exec SchemaDocument,
    pub document: &'exec Document,
    pub validation_plan: &'exec mut ValidationPlan,
    pub errors: &'exec mut Option<Vec<ValidationError>>
}

impl<'exec> StartPayload<OnGraphQLValidationEndPayload<'exec>> for OnGraphQLValidationStartPayload<'exec> {}

pub struct OnGraphQLValidationEndPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub graphql_params: &'exec GraphQLParams,
    pub schema: &'exec SchemaDocument,
    pub document: &'exec Document,
    pub errors: &'exec mut Vec<ValidationError>,
}

impl<'exec> EndPayload for OnGraphQLValidationEndPayload<'exec> {}