use ntex::{
    http::Response,
    web::{self, DefaultError, WebRequest},
};

use crate::{
    plugin_context::PluginContext,
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnHttpRequestHookPayload<'req> {
    pub router_http_request: WebRequest<DefaultError>,
    pub context: &'req PluginContext,
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
}

impl<'req> OnHttpResponseHookPayload<'req> {
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
