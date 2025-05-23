use error::QueryPlanError;
use fetch::{error::FetchGraphError, fetch_graph::build_fetch_graph_from_query_tree};
use graphql_parser::{query, schema};
use plan_nodes::QueryPlan;
use query_plan::build_query_plan_from_fetch_graph;
use tree::{paths_to_trees, query_tree::QueryTree};
use walker::{error::WalkOperationError, walk_operation};

use crate::{
    graph::{error::GraphError, Graph},
    state::supergraph_state::SupergraphState,
    utils::operation_utils::get_operation_to_execute,
};

mod error;
pub mod fetch;
pub mod plan_nodes;
pub mod query_plan;
pub mod tree;
pub mod walker;

pub struct Planner {
    graph: Graph,
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
        let graph = Graph::graph_from_supergraph_state(&supergraph_state)?;

        Ok(Planner { graph })
    }

    pub fn plan(
        &self,
        operation_document: &query::Document<'static, String>,
    ) -> Result<QueryPlan, PlannerError> {
        let operation = get_operation_to_execute(operation_document)
            .ok_or(PlannerError::MissingOperationToExecute)?;
        let best_paths_per_leaf = walk_operation(&self.graph, operation)?;
        let qtps = paths_to_trees(&self.graph, &best_paths_per_leaf)?;
        let query_tree = QueryTree::merge_trees(qtps);
        let fetch_graph = build_fetch_graph_from_query_tree(&self.graph, query_tree)?;
        let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

        Ok(query_plan)
    }
}
