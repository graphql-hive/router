use std::collections::HashMap;

use bytes::Bytes;
use hive_router_query_planner::ast::operation::SubgraphFetchOperation;
use ntex::web::HttpRequest;

use crate::{executors::dedupe::SharedResponse, response::{graphql_error::GraphQLError, value::Value}};



pub struct OnSubgraphExecuteStartPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
	pub subgraph_name: &'exec str,
	// The node that initiates this subgraph execution
	pub execution_request: &'exec mut SubgraphExecutionRequest<'exec>,
	// This will be tricky to implement with the current structure,
	// but I'm sure we'll figure it out
	pub response: &'exec mut Option<SubgraphResponse<'exec>>,
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

pub struct SubgraphResponse<'exec> {
    pub data: Value<'exec>,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<HashMap<String, Value<'exec>>>,
}

pub struct OnSubgraphExecuteEndPayload<'exec> {
    pub router_http_request: &'exec HttpRequest,
    pub subgraph_name: &'exec str,
    // The node that initiates this subgraph execution
    pub execution_request: &'exec SubgraphExecutionRequest<'exec>,
    // This will be tricky to implement with the current structure,
    // but I'm sure we'll figure it out
    pub response: &'exec SubgraphResponse<'exec>,
}
