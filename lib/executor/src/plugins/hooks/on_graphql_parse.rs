use graphql_tools::static_graphql::query::Document;
use ntex::http::Response;

use crate::{
    hooks::on_graphql_params::GraphQLParams,
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnGraphQLParseStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub graphql_params: &'exec GraphQLParams,
    // Override
    pub document: Option<Document>,
}

impl<'exec> OnGraphQLParseStartHookPayload<'exec> {
    pub fn with_parsed_document(mut self, document: Document) -> Self {
        self.document = Some(document);
        self
    }
}

impl<'exec> StartHookPayload<OnGraphQLParseEndHookPayload, Response>
    for OnGraphQLParseStartHookPayload<'exec>
{
}

pub type OnGraphQLParseHookResult<'exec> = StartHookResult<
    'exec,
    OnGraphQLParseStartHookPayload<'exec>,
    OnGraphQLParseEndHookPayload,
    Response,
>;

pub struct OnGraphQLParseEndHookPayload {
    pub document: Document,
}

impl EndHookPayload<Response> for OnGraphQLParseEndHookPayload {}

pub type OnGraphQLParseEndHookResult = EndHookResult<OnGraphQLParseEndHookPayload, Response>;
