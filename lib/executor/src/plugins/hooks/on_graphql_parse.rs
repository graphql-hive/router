use graphql_tools::static_graphql::query::Document;

use crate::{hooks::on_deserialization::GraphQLParams, plugin_trait::{EndPayload, StartPayload}};

pub struct OnGraphQLParseStartPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub graphql_params: &'exec GraphQLParams,
    pub document: Option<Document>,
}

impl<'exec> StartPayload<OnGraphQLParseEndPayload<'exec>> for OnGraphQLParseStartPayload<'exec> {}

pub struct OnGraphQLParseEndPayload<'exec> {
    pub router_http_request: &'exec ntex::web::HttpRequest,
    pub graphql_params: &'exec GraphQLParams,
    pub document: Document,
}

impl<'exec> EndPayload for OnGraphQLParseEndPayload<'exec> {}