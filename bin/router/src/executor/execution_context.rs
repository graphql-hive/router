use bytes::Bytes;
use std::collections::HashMap;

use crate::query_planner::planner::plan_nodes::ResponsePathRef;

use crate::executor::{
    execution::demand_control::subgraph_response_tracker::SubgraphResponseCostTracker,
    extensions::aggregator::ExtensionsAggregator,
    headers::response::ResponseHeaderAggregator,
    response::{
        graphql_error::{GraphQLError, GraphQLErrorPath},
        storage::ResponsesStorage,
        value::Value,
    },
};

pub struct ExecutionContext<'a> {
    pub response_storage: ResponsesStorage,
    pub data: Value<'a>,
    pub errors: Vec<GraphQLError>,
    pub response_headers_aggregator: ResponseHeaderAggregator,
    pub extensions_aggregator: ExtensionsAggregator,
    pub subgraph_response_cost_tracker: SubgraphResponseCostTracker<'a>,
    /// The body the data started from borrows from (see `read_start_data`, `keep_body`)
    pub(crate) start_json: Option<Bytes>,
}

impl<'a> Default for ExecutionContext<'a> {
    fn default() -> Self {
        ExecutionContext {
            response_storage: Default::default(),
            errors: Vec::new(),
            data: Value::Null,
            response_headers_aggregator: Default::default(),
            extensions_aggregator: Default::default(),
            subgraph_response_cost_tracker: SubgraphResponseCostTracker::new(),
            start_json: None,
        }
    }
}

impl<'a> ExecutionContext<'a> {
    /// Keeps the body the starting data borrows from, a subscription event's
    pub fn keep_body(&mut self, body: Option<Bytes>) {
        self.start_json = body;
    }

    /// The data the plan starts from, read from its JSON object (an introspection result, or
    /// what a plugin put in). The values borrow from `json`, which the context keeps. JSON that
    /// isn't an object is left out, like `{}`.
    pub fn read_start_data(&mut self, json: Option<Bytes>) -> Value<'a> {
        let Some(json) = json else {
            return Value::Object(Vec::new());
        };
        // SAFETY: `json` is a refcounted buffer that doesn't move; the context keeps it in
        // `start_json` for as long as the data, like `SubgraphResponse::deserialize_from_bytes`
        // keeps the body its data borrows from
        let bytes: &'static [u8] = unsafe { std::mem::transmute::<&[u8], _>(json.as_ref()) };
        self.start_json = Some(json);
        match sonic_rs::from_slice::<Value<'static>>(bytes) {
            Ok(data @ Value::Object(_)) => data,
            _ => {
                tracing::warn!(
                    "the data a plugin set before execution isn't a JSON object, it's left out"
                );
                Value::Object(Vec::new())
            }
        }
    }

    pub fn new(data: Value<'a>, errors: Vec<GraphQLError>) -> Self {
        ExecutionContext {
            data,
            errors,
            ..Default::default()
        }
    }

    pub fn handle_errors(
        &mut self,
        subgraph_name: &str,
        affected_path: Option<ResponsePathRef<'_>>,
        errors: Option<Vec<GraphQLError>>,
        entity_index_error_map: Option<HashMap<usize, Vec<GraphQLErrorPath>>>,
    ) {
        if let Some(response_errors) = errors {
            let affected_path = affected_path.map(|path| path.to_string());
            for response_error in response_errors {
                let mut processed_error = response_error.add_subgraph_name(subgraph_name);

                if let Some(affected_path) = &affected_path {
                    processed_error = processed_error.add_affected_path(affected_path.clone());
                }

                if let Some(entity_index_error_map) = &entity_index_error_map {
                    let normalized_errors =
                        processed_error.normalize_entity_error(entity_index_error_map);
                    self.errors.extend(normalized_errors);
                } else {
                    self.errors.push(processed_error);
                }
            }
        }
    }
}
