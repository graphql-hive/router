use std::collections::HashMap;

use bumpalo::Bump;

use hive_router_query_planner::planner::slot_path::{slot_path_to_string, SlotPathSegment};

use crate::{
    execution::demand_control::subgraph_response_tracker::SubgraphResponseCostTracker,
    extensions::aggregator::ExtensionsAggregator,
    headers::response::ResponseHeaderAggregator,
    response::{
        arena::ResponseArena,
        graphql_error::{GraphQLError, GraphQLErrorPath},
        storage::ResponsesStorage,
        subgraph_response::SubgraphResponse,
        value::Value,
    },
};

pub struct ExecutionContext<'a> {
    pub response_storage: ResponsesStorage,
    /// Arena for values the executor builds itself — the root object, introspection results,
    /// anything a merge has to materialize. Subgraph responses bring their own, which end up
    /// in `response_storage`.
    pub arena: ResponseArena,
    pub data: Value<'static>,
    pub errors: Vec<GraphQLError>,
    pub response_headers_aggregator: ResponseHeaderAggregator,
    pub extensions_aggregator: ExtensionsAggregator,
    pub subgraph_response_cost_tracker: SubgraphResponseCostTracker<'a>,
}

impl<'a> Default for ExecutionContext<'a> {
    fn default() -> Self {
        ExecutionContext {
            response_storage: Default::default(),
            arena: ResponseArena::new(),
            errors: Vec::new(),
            data: Value::Null,
            response_headers_aggregator: Default::default(),
            extensions_aggregator: Default::default(),
            subgraph_response_cost_tracker: SubgraphResponseCostTracker::new(),
        }
    }
}

impl<'a> ExecutionContext<'a> {
    /// `arena` must be the one `data` was built in — the context owns it from here, and the
    /// root object would dangle otherwise.
    pub fn new(data: Value<'static>, errors: Vec<GraphQLError>, arena: ResponseArena) -> Self {
        ExecutionContext {
            data,
            errors,
            arena,
            ..Default::default()
        }
    }

    /// Takes over what a subgraph response's values borrow — its buffer and its arena — and
    /// hands back the arena, for the subtrees a merge still has to materialize.
    ///
    /// Called once per response, before anything is merged out of it: from here on the tree
    /// stays valid for the rest of the request, and is freed in a few chunk frees when this
    /// context drops.
    pub fn absorb(&mut self, response: &mut SubgraphResponse<'static>) -> &'static Bump {
        if let Some(bytes) = response.bytes.take() {
            self.response_storage.add_response(bytes);
        }
        let arena = response.arena.take().unwrap_or_default();
        let bump = arena.borrow_unbounded();
        self.response_storage.add_arena(arena);
        bump
    }

    pub fn handle_errors(
        &mut self,
        subgraph_name: &str,
        affected_path: Option<&[SlotPathSegment]>,
        errors: Option<Vec<GraphQLError>>,
        entity_index_error_map: Option<HashMap<&usize, Vec<GraphQLErrorPath>>>,
    ) {
        if let Some(response_errors) = errors {
            let affected_path = affected_path.map(slot_path_to_string);
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
