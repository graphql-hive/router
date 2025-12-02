use graphql_tools::static_graphql::query::Document;

use crate::{
    hooks::on_graphql_params::GraphQLParams,
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnGraphQLParseStartHookPayload<'exec> {
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    pub context: &'exec PluginContext,
    pub graphql_params: &'exec GraphQLParams,
    pub document: Option<Document>,
}

impl<'exec> StartHookPayload<OnGraphQLParseEndHookPayload>
    for OnGraphQLParseStartHookPayload<'exec>
{
}

pub type OnGraphQLParseHookResult<'exec> =
    StartHookResult<'exec, OnGraphQLParseStartHookPayload<'exec>, OnGraphQLParseEndHookPayload>;

pub struct OnGraphQLParseEndHookPayload {
    pub document: Document,
}

impl EndHookPayload for OnGraphQLParseEndHookPayload {}

pub type OnGraphQLParseEndHookResult = EndHookResult<OnGraphQLParseEndHookPayload>;
