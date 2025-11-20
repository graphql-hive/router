use ntex::web::{self, DefaultError};

use crate::{
    plugin_context::PluginContext,
    plugin_trait::{EndPayload, StartPayload},
};

pub struct OnHttpRequestPayload<'req> {
    pub router_http_request: web::WebRequest<DefaultError>,
    pub context: &'req PluginContext,
    pub response: Option<web::WebResponse>,
}

impl<'req> StartPayload<OnHttpResponsePayload> for OnHttpRequestPayload<'req> {}

pub struct OnHttpResponsePayload {
    pub response: web::WebResponse,
}

impl EndPayload for OnHttpResponsePayload {}
