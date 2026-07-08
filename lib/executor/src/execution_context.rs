use std::collections::HashMap;
use std::sync::Arc;

use hive_router_query_planner::planner::plan_nodes::FlattenNodePath;

use crate::{
    execution::demand_control::subgraph_response_tracker::SubgraphResponseCostTracker,
    extensions::aggregator::ExtensionsAggregator,
    headers::response::ResponseHeaderAggregator,
    response::{
        flat_store::{FlatResponseStore, FlatValueId, ResponseKeys},
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
    pub extensions_aggregator: ExtensionsAggregator<'a>,
    pub subgraph_response_cost_tracker: SubgraphResponseCostTracker<'a>,
    pub flat_store: Option<FlatResponseStore<'static>>,
    pub flat_keys: Option<Arc<ResponseKeys>>,
    pub flat_root: Option<FlatValueId>,
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
            flat_store: None,
            flat_keys: None,
            flat_root: None,
        }
    }
}

impl<'a> ExecutionContext<'a> {
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
        affected_path: Option<&FlattenNodePath>,
        errors: Option<Vec<GraphQLError>>,
        entity_index_error_map: Option<HashMap<&usize, Vec<GraphQLErrorPath>>>,
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
