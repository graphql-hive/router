use std::collections::HashMap;

use hive_router_query_planner::ast::operation::SubgraphFetchOperation;
use http::{HeaderMap, Uri};
use ntex::web::HttpRequest;

use crate::
    executors::dedupe::SharedResponse
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

pub struct SubgraphExecutionRequest<'exec> {
    pub query: &'exec str,
    // We can add the original operation here too
    pub operation: &'exec SubgraphFetchOperation,

    pub dedupe: bool,
    pub operation_name: Option<&'exec str>,
    pub variables: Option<HashMap<&'exec str, &'exec sonic_rs::Value>>,
    pub extensions: Option<HashMap<String, sonic_rs::Value>>,
    pub representations: Option<Vec<u8>>,
}

pub struct OnSubgraphHttpResponsePayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub subgraph_name: &'exec str,
    // The node that initiates this subgraph execution
    pub execution_request: &'exec SubgraphExecutionRequest<'exec>,
    // This will be tricky to implement with the current structure,
    // but I'm sure we'll figure it out
	pub response: &'exec mut SharedResponse,
}
