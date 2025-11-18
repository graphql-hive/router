use bytes::Bytes;
use http::{HeaderMap, Request, Uri};
use http_body_util::Full;
use ntex::web::HttpRequest;

use crate::{
    executors::{common::SubgraphExecutionRequest, dedupe::SharedResponse}, plugin_trait::{EndPayload, StartPayload}}
;

pub struct OnSubgraphHttpRequestPayload<'exec> {
    pub subgraph_name: &'exec str,
    // At this point, there is no point of mutating this
    pub request: Request<Full<Bytes>>,

    // Early response
    pub response: Option<SharedResponse>,
}

impl<'exec> StartPayload<OnSubgraphHttpResponsePayload> for OnSubgraphHttpRequestPayload<'exec> {}

pub struct OnSubgraphHttpResponsePayload {
	pub response: SharedResponse,
}

impl<'exec> EndPayload for OnSubgraphHttpResponsePayload {}
