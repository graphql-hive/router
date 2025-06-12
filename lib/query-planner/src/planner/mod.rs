use error::QueryPlanError;
use fetch::{error::FetchGraphError, fetch_graph::build_fetch_graph_from_query_tree};
use graphql_parser::schema;
use plan_nodes::QueryPlan;
use query_plan::build_query_plan_from_fetch_graph;
use walker::{error::WalkOperationError, walk_operation};

use crate::{
    ast::operation::OperationDefinition,
    consumer_schema::ConsumerSchema,
    graph::{error::GraphError, Graph},
    planner::best::find_best_combination,
    state::supergraph_state::SupergraphState,
};

pub mod best;
mod error;
pub mod fetch;
pub mod plan_nodes;
pub mod query_plan;
pub mod tree;
pub mod walker;

pub struct Planner {
    graph: Graph,
    pub supergraph: SupergraphState,
    pub consumer_schema: ConsumerSchema,
}

#[derive(Debug, thiserror::Error)]
pub enum PlannerError {
    #[error("failed to initalize relations graph: {0}")]
    GraphInitError(Box<GraphError>),
    #[error("failed to locate operation to execute")]
    MissingOperationToExecute,
    #[error("walker failed to locate path: {0}")]
    PathLocatorError(Box<WalkOperationError>),
    #[error("failed to build fetch graph: {0}")]
    FailedToConstructFetchGraph(Box<FetchGraphError>),
    #[error("failed to build plan: {0}")]
    QueryPlanBuildFailed(Box<QueryPlanError>),
}

impl From<GraphError> for PlannerError {
    fn from(value: GraphError) -> Self {
        PlannerError::GraphInitError(Box::new(value))
    }
}

impl From<WalkOperationError> for PlannerError {
    fn from(value: WalkOperationError) -> Self {
        PlannerError::PathLocatorError(Box::new(value))
    }
}

impl From<FetchGraphError> for PlannerError {
    fn from(value: FetchGraphError) -> Self {
        PlannerError::FailedToConstructFetchGraph(Box::new(value))
    }
}

impl From<QueryPlanError> for PlannerError {
    fn from(value: QueryPlanError) -> Self {
        PlannerError::QueryPlanBuildFailed(Box::new(value))
    }
}

impl Planner {
    pub fn new_from_supergraph(
        parsed_supergraph: &schema::Document<'static, String>,
    ) -> Result<Self, PlannerError> {
        let supergraph_state = SupergraphState::new(parsed_supergraph);
        Self::new_from_supergraph_state(supergraph_state, parsed_supergraph)
    }

    pub fn new_from_supergraph_state(
        supergraph_state: SupergraphState,
        parsed_supergraph: &schema::Document<'static, String>,
    ) -> Result<Self, PlannerError> {
        let graph = Graph::graph_from_supergraph_state(&supergraph_state)?;
        let consumer_schema = ConsumerSchema::new_from_supergraph(parsed_supergraph);

        Ok(Planner {
            graph,
            consumer_schema,
            supergraph: supergraph_state,
        })
    }

    pub fn plan_from_normalized_operation(
        &self,
        normalized_operation: &OperationDefinition,
    ) -> Result<QueryPlan, PlannerError> {
        let best_paths_per_leaf = walk_operation(&self.graph, normalized_operation)?;
        let query_tree = find_best_combination(&self.graph, best_paths_per_leaf).unwrap();
        let fetch_graph = build_fetch_graph_from_query_tree(&self.graph, query_tree)?;
        let query_plan = build_query_plan_from_fetch_graph(fetch_graph)?;

        Ok(query_plan)
    }
}
