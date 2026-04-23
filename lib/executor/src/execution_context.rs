use std::collections::HashMap;

use hive_router_query_planner::planner::plan_nodes::FlattenNodePath;

use crate::{
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
}

impl<'a> Default for ExecutionContext<'a> {
    fn default() -> Self {
        ExecutionContext {
            response_storage: Default::default(),
            errors: Vec::new(),
            data: Value::Null,
            response_headers_aggregator: Default::default(),
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
