use ntex::{http::Response, web::HttpRequest};

use crate::plugin_trait::{EndPayload, StartPayload};

pub struct OnHttpRequestPayload<'exec> {
    pub client_request: &'exec HttpRequest,
}

impl<'exec> StartPayload<OnHttpResponse<'exec>> for OnHttpRequestPayload<'exec> {}

pub struct OnHttpResponse<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub response: &'exec mut Response,
}

impl<'exec> EndPayload for OnHttpResponse<'exec> {}
