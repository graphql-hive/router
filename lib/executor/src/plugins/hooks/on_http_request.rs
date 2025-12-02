use ntex::web::{self, DefaultError, WebRequest};

use crate::{
    plugin_context::PluginContext,
    plugin_trait::{EndHookPayload, EndHookResult, StartHookPayload, StartHookResult},
};

pub struct OnHttpRequestHookPayload<'req> {
    pub router_http_request: WebRequest<DefaultError>,
    pub context: &'req PluginContext,
}

impl<'req> StartHookPayload<OnHttpResponseHookPayload<'req>> for OnHttpRequestHookPayload<'req> {}

pub type OnHttpRequestHookResult<'req> =
    StartHookResult<'req, OnHttpRequestHookPayload<'req>, OnHttpResponseHookPayload<'req>>;

pub struct OnHttpResponseHookPayload<'req> {
    pub response: web::WebResponse,
    pub context: &'req PluginContext,
}

impl<'req> EndHookPayload for OnHttpResponseHookPayload<'req> {}

pub type OnHttpResponseHookResult<'req> = EndHookResult<OnHttpResponseHookPayload<'req>>;
