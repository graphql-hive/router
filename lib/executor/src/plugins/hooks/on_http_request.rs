use ntex::web::{self, DefaultError, WebRequest};

use crate::{
    plugin_context::PluginContext,
    plugin_trait::{EndPayload, StartPayload},
};

pub struct OnHttpRequestPayload<'req> {
    pub router_http_request: WebRequest<DefaultError>,
    pub context: &'req PluginContext,
}

impl<'req> StartPayload<OnHttpResponsePayload<'req>> for OnHttpRequestPayload<'req> {}

pub struct OnHttpResponsePayload<'req> {
    pub response: web::WebResponse,
    pub context: &'req PluginContext,
}

impl<'req> EndPayload for OnHttpResponsePayload<'req> {}
