use graphql_tools::static_graphql::query::Document;

use crate::{
    hooks::on_graphql_params::GraphQLParams,
    plugin_trait::{EndPayload, StartPayload},
};

pub struct OnGraphQLParseStartPayload<'exec> {
    pub router_http_request: ntex::web::HttpRequest,
    pub graphql_params: &'exec GraphQLParams,
    pub document: Option<Document>,
}

impl<'exec> StartPayload<OnGraphQLParseEndPayload> for OnGraphQLParseStartPayload<'exec> {}

pub struct OnGraphQLParseEndPayload {
    pub document: Document,
}

impl EndPayload for OnGraphQLParseEndPayload {}
