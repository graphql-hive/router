use std::sync::Arc;

use graphql_tools::static_graphql::query::Document;
use ntex::http::Response;

use crate::{
    hooks::on_graphql_params::GraphQLParams,
    plugin_context::{PluginContext, RouterHttpRequest},
    plugin_trait::{CacheHint, EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnGraphQLParseStartHookPayload<'exec> {
    /// The incoming HTTP request to the router for which the GraphQL execution is happening.
    /// It includes all the details of the request such as headers, body, etc.
    ///
    /// Example:
    /// ```
    ///  let my_header = payload.router_http_request.headers.get("my-header");
    ///  // do something with the header...
    ///  payload.proceed()
    /// ```
    pub router_http_request: &'exec RouterHttpRequest<'exec>,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#context-data-sharing)
    pub context: &'exec PluginContext,
    /// The GraphQL parameters parsed from the HTTP request body by the router.
    /// This includes the `query`, `operationName`, `variables`, and `extensions`
    /// [Learn more about GraphQL-over-HTTP params](https://graphql.org/learn/serving-over-http/#request-format)
    pub graphql_params: &'exec GraphQLParams,
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
    /// Parsed GraphQL document from the query string in the GraphQL parameters.
    /// It contains the Abstract Syntax Tree (AST) representation of the GraphQL query, mutation, or subscription
    /// sent by the client in the request body.
    pub document: Arc<Document>,
    /// The cache hint for the parsed GraphQL document.
    /// - If this is `CacheHint::Hit`, it means the parsing process didn't happen because the result was retrieved from the cache.
    /// - If this is `CacheHint::Miss`, it means the parsing process happened and the result was not retrieved from the cache.
    pub cache_hint: CacheHint,
}

impl EndHookPayload<Response> for OnGraphQLParseEndHookPayload {}

pub type OnGraphQLParseEndHookResult = EndHookResult<OnGraphQLParseEndHookPayload, Response>;
