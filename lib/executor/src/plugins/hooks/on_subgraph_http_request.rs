use bytes::Bytes;
use http::Request;
use http_body_util::Full;

use crate::{
    executors::dedupe::SharedResponse,
    plugin_trait::{EndPayload, StartPayload},
};

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

impl EndPayload for OnSubgraphHttpResponsePayload {}
