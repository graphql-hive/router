use error::QueryPlanError;
use fetch::{error::FetchGraphError, fetch_graph::build_fetch_graph_from_query_tree};
use graphql_parser::{query, schema};
use plan_nodes::QueryPlan;
use query_plan::build_query_plan_from_fetch_graph;
use tree::{paths_to_trees, query_tree::QueryTree};
use walker::{error::WalkOperationError, walk_operation};

use crate::{
    ast::operation::OperationDefinition,
    consumer_schema::ConsumerSchema,
    graph::{error::GraphError, Graph},
    state::supergraph_state::SupergraphState,
    utils::operation_utils::prepare_document,
};

mod error;
pub mod fetch;
pub mod plan_nodes;
pub mod query_plan;
pub mod tree;
pub mod walker;

pub struct Planner {
    graph: Graph,
    pub consumer_schema: ConsumerSchema,
}

#[derive(Debug, thiserror::Error)]
pub enum PlannerError {
    #[error("failed to initalize relations graph: {0}")]
    GraphInitError(GraphError),
    #[error("failed to locate operation to execute")]
    MissingOperationToExecute,
    #[error("walker failed to locate path: {0}")]
    PathLocatorError(WalkOperationError),
    #[error("failed to build fetch graph: {0}")]
    FailedToConstructFetchGraph(FetchGraphError),
    #[error("failed to build plan: {0}")]
    QueryPlanBuildFailed(QueryPlanError),
}

impl From<GraphError> for PlannerError {
    fn from(value: GraphError) -> Self {
        PlannerError::GraphInitError(value)
    }
}

impl From<WalkOperationError> for PlannerError {
    fn from(value: WalkOperationError) -> Self {
        PlannerError::PathLocatorError(value)
    }
}

impl From<FetchGraphError> for PlannerError {
    fn from(value: FetchGraphError) -> Self {
        PlannerError::FailedToConstructFetchGraph(value)
    }
}

impl From<QueryPlanError> for PlannerError {
    fn from(value: QueryPlanError) -> Self {
        PlannerError::QueryPlanBuildFailed(value)
    }
}

impl Planner {
    pub fn new_from_supergraph(
        parsed_supergraph: &schema::Document<'static, String>,
    ) -> Result<Self, PlannerError> {
        let supergraph_state = SupergraphState::new(parsed_supergraph);
        Self::new_from_supergraph_state(&supergraph_state)
    }

    pub fn new_from_supergraph_state(
        supergraph_state: &SupergraphState,
    ) -> Result<Self, PlannerError> {
        let graph = Graph::graph_from_supergraph_state(supergraph_state)?;
        let consumer_schema = ConsumerSchema::new_from_supergraph(supergraph_state.document);

        Ok(Planner {
            graph,
            consumer_schema,
        })
    }

    pub fn plan(
        &self,
        operation_document: query::Document<'static, String>,
        operation_name: Option<&str>,
    ) -> Result<QueryPlan, PlannerError> {
        let document = prepare_document(operation_document, operation_name);
        let operation = document
            .executable_operation()
            .ok_or(PlannerError::MissingOperationToExecute)?;
        self.plan_from_normalized_operation(operation)
    }

    pub fn plan_from_normalized_operation(
        &self,
        normalized_operation: &OperationDefinition,
    ) -> Result<QueryPlan, PlannerError> {
        let best_paths_per_leaf = walk_operation(&self.graph, normalized_operation)?;
        let qtps = paths_to_trees(&self.graph, &best_paths_per_leaf)?;
        let query_tree = QueryTree::merge_trees(qtps);
        let fetch_graph = build_fetch_graph_from_query_tree(&self.graph, query_tree)?;
        let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

        Ok(query_plan)
    }
}
