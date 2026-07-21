use ntex::{
    http::Response,
    web::{self, DefaultError, WebRequest},
};
use std::sync::Arc;

use crate::{
    hooks::on_supergraph_load::Supergraph,
    plugin_context::PluginContext,
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
    request_context::RequestContextPluginApi,
};

type RequestContextApi = RequestContextPluginApi<super::OnHttpRequest>;

pub struct OnHttpRequestHookPayload<'req> {
    /// The raw incoming HTTP request to the router
    /// It includes all the details of the request such as headers, body, etc.
    ///
    /// Example:
    /// ```
    /// use hive_router::{
    ///    plugins::hooks::on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
    /// };
    ///
    /// fn on_http_request<'req>(mut payload: OnHttpRequestHookPayload<'req>) -> OnHttpRequestHookResult<'req> {
    ///     let my_header = payload.router_http_request.headers().get("my-header");
    ///     // do something with the header...
    ///     payload.proceed()
    /// }
    /// ```
    pub router_http_request: WebRequest<DefaultError>,
    /// The context object that can be used to share data across different plugin hooks for the same request.
    /// It is unique per request and is dropped after the response is sent.
    ///
    /// [Learn more about the context data sharing in the docs](https://the-guild.dev/graphql/hive/docs/router/extensibility/plugin_system#context-data-sharing)
    ///
    /// Example:
    /// ```
    /// use hive_router::{
    ///     plugins::hooks::{
    ///         on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
    ///         on_execute::{OnExecuteStartHookPayload, OnExecuteStartHookResult}
    ///     },
    ///     plugin_context::PluginContext,
    ///     async_trait::async_trait,
    /// };
    ///
    /// struct ContextData {
    ///     greetings: String
    /// }
    ///
    /// #[async_trait]
    /// impl RouterPlugin for MyPlugin {
    ///     fn on_http_request<'req>(mut payload: OnHttpRequestHookPayload<'req>) -> OnHttpRequestHookResult<'req> {
    ///         let context_data = ContextData {
    ///            greetings: "Hello from context!".to_string()
    ///         };
    ///
    ///        payload.context.insert(context_data);
    ///
    ///        payload.proceed()
    ///     }
    ///
    ///     async fn on_execute<'exec>(&'exec self, payload: OnExecuteStartHookPayload<'exec>) -> OnExecuteStartHookResult<'exec> {
    ///         let context_data = payload.context.get::<ContextData>().unwrap();
    ///         println!("{}", context_data.greetings); // prints "Hello from context!"
    ///         payload.proceed()
    ///     }
    /// }
    /// ```
    pub context: &'req PluginContext,
    pub request_context: RequestContextApi,
}

impl<'req> OnHttpRequestHookPayload<'req> {
    /// Selects the supergraph used for this request. Set this to swap the entire GraphQL schema
    /// (and everything derived from it: validation, introspection, planning, execution) for a
    /// different one, on a per-request basis, e.g. to serve a different set of fields depending
    /// on the caller.
    ///
    /// The plugin owns construction, retention, replacement and removal of every
    /// `Arc<Supergraph>` it selects from. This method stores a cheap, read-only snapshot in
    /// request extensions and never retains the owner `Arc` itself. A later plugin can replace
    /// that snapshot, preserving normal last-write-wins hook behavior. This means a
    /// plugin can drop or replace a variant at any time: new requests simply stop being able to
    /// select it, existing in-flight requests already holding a snapshot finish normally, and any
    /// active subscriptions selected from it are closed with a schema-reload error.
    ///
    /// ### Example
    ///
    /// ```ignore
    /// payload.set_supergraph(supergraph.clone());
    /// ```
    pub fn set_supergraph(&self, supergraph: Arc<Supergraph>) {
        self.router_http_request
            .extensions_mut()
            .insert(supergraph.snapshot());
    }
}

impl<'req> StartHookPayload<OnHttpResponseHookPayload<'req>, Response>
    for OnHttpRequestHookPayload<'req>
{
}

pub type OnHttpRequestHookResult<'req> = StartHookResult<
    'req,
    OnHttpRequestHookPayload<'req>,
    OnHttpResponseHookPayload<'req>,
    Response,
>;

pub struct OnHttpResponseHookPayload<'req> {
    pub response: web::WebResponse,
    pub context: &'req PluginContext,
    pub request_context: RequestContextApi,
}

impl<'req> OnHttpResponseHookPayload<'req> {
    /// Manipulate the outgoing HTTP response before it's sent to the client.
    /// This can be used to modify headers, change the body, etc.
    ///
    /// Example:
    /// ```
    /// fn on_http_request<'req>(
    ///     &'req self,
    ///     payload: OnHttpRequestHookPayload<'req>,
    /// ) -> OnHttpRequestHookResult<'req> {
    ///     payload.on_end(|payload| {
    ///         payload.map_response(|mut response| {
    ///             response.response_mut().headers_mut().insert(
    ///                 "x-served-by",
    ///                 "hive-router".parse().unwrap(),
    ///             );
    ///             response
    ///         }).proceed()
    ///     })
    /// }
    /// ```
    pub fn map_response<F>(mut self, f: F) -> Self
    where
        F: FnOnce(web::WebResponse) -> web::WebResponse,
    {
        self.response = f(self.response);
        self
    }
}

impl<'req> EndHookPayload<Response> for OnHttpResponseHookPayload<'req> {}

pub type OnHttpResponseHookResult<'req> = EndHookResult<OnHttpResponseHookPayload<'req>, Response>;
