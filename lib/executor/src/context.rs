use std::collections::HashMap;

use hive_router_query_planner::planner::plan_nodes::{FetchNode, FetchRewrite, QueryPlan};

use crate::{
    headers::plan::ResponseHeaderAggregator,
    response::{
        graphql_error::{GraphQLError, GraphQLErrorPathSegment},
        storage::ResponsesStorage,
        value::Value,
    },
};

pub struct ExecutionContext<'a> {
    pub response_storage: ResponsesStorage,
    pub final_response: Value<'a>,
    pub errors: Vec<GraphQLError>,
    pub output_rewrites: OutputRewritesStorage,
    pub response_headers_aggregator: ResponseHeaderAggregator,
}

impl<'a> Default for ExecutionContext<'a> {
    fn default() -> Self {
        ExecutionContext {
            response_storage: Default::default(),
            output_rewrites: Default::default(),
            errors: Vec::new(),
            final_response: Value::Null,
            response_headers_aggregator: Default::default(),
        }
    }
}

impl<'a> ExecutionContext<'a> {
    pub fn new(query_plan: &QueryPlan, init_final_response: Value<'a>) -> Self {
        ExecutionContext {
            response_storage: ResponsesStorage::new(),
            output_rewrites: OutputRewritesStorage::from_query_plan(query_plan),
            errors: Vec::new(),
            final_response: init_final_response,
            response_headers_aggregator: Default::default(),
        }
    }

    pub fn handle_errors(
        &mut self,
        errors: Option<Vec<GraphQLError>>,
        entity_index_error_map: Option<HashMap<&usize, Vec<Vec<GraphQLErrorPathSegment>>>>,
    ) {
        if let Some(response_errors) = errors {
            for response_error in response_errors {
                match &response_error.path.as_deref() {
                    Some(
                        [GraphQLErrorPathSegment::String(maybe_entities), GraphQLErrorPathSegment::Index(entity_index), rest_of_path @ ..],
                    ) if maybe_entities == "_entities" => {
                        if let Some(entity_error_paths) = entity_index_error_map
                            .as_ref()
                            .and_then(|m| m.get(entity_index))
                        {
                            for entity_error_path in entity_error_paths {
                                let mut new_error_path = entity_error_path.clone();
                                if !rest_of_path.is_empty() {
                                    new_error_path.extend_from_slice(rest_of_path);
                                }
                                self.errors.push(GraphQLError {
                                    path: Some(new_error_path),
                                    ..response_error.clone()
                                });
                            }
                        } else {
                            // If we don't have the entity index in the map, just push the original error
                            // without any path, because `_entities` is not a valid path in the final response.
                            self.errors.push(GraphQLError {
                                path: None,
                                ..response_error
                            });
                        }
                    }
                    _ => {
                        self.errors.push(response_error);
                    }
                }
            }
        }
    }
}

#[derive(Default)]
pub struct OutputRewritesStorage {
    output_rewrites: HashMap<i64, Vec<FetchRewrite>>,
}

impl OutputRewritesStorage {
    pub fn from_query_plan(query_plan: &QueryPlan) -> OutputRewritesStorage {
        let mut output_rewrites = OutputRewritesStorage {
            output_rewrites: HashMap::new(),
        };

        for fetch_node in query_plan.fetch_nodes() {
            output_rewrites.add_maybe(fetch_node);
        }

        output_rewrites
    }

    fn add_maybe(&mut self, fetch_node: &FetchNode) {
        if let Some(rewrites) = &fetch_node.output_rewrites {
            self.output_rewrites.insert(fetch_node.id, rewrites.clone());
        }
    }

    pub fn get(&self, id: i64) -> Option<&Vec<FetchRewrite>> {
        self.output_rewrites.get(&id)
    }
}
