use http::{HeaderMap, Uri};
use ntex::web::HttpRequest;

use crate::{
    executors::{common::SubgraphExecutionRequest, dedupe::SharedResponse}, plugin_trait::{EndPayload, StartPayload}}
;

pub struct OnSubgraphHttpRequestPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub subgraph_name: &'exec str,
    // At this point, there is no point of mutating this
    pub execution_request: &'exec SubgraphExecutionRequest<'exec>,

    pub endpoint: &'exec mut Uri,
    // By default, it is POST
    pub method: &'exec mut http::Method,
    pub headers: &'exec mut HeaderMap,
    pub request_body: &'exec mut Vec<u8>,

    // Early response
    pub response: &'exec mut Option<SharedResponse>,
}

impl<'exec> StartPayload<OnSubgraphHttpResponsePayload<'exec>> for OnSubgraphHttpRequestPayload<'exec> {}

pub struct OnSubgraphHttpResponsePayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub subgraph_name: &'exec str,
    pub execution_request: &'exec SubgraphExecutionRequest<'exec>,
	pub response: &'exec mut SharedResponse,
}

impl<'exec> EndPayload for OnSubgraphHttpResponsePayload<'exec> {}
