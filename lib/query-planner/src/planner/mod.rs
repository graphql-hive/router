use error::QueryPlanError;
use fetch::{error::FetchGraphError, fetch_graph::build_fetch_graph_from_query_tree};
use graphql_parser::schema;
use petgraph::graph::NodeIndex;
use plan_nodes::QueryPlan;
use query_plan::build_query_plan_from_fetch_graph;
use walker::{error::WalkOperationError, walk_operation};

use crate::{
    ast::operation::{OperationDefinition, VariableDefinition},
    consumer_schema::ConsumerSchema,
    graph::{edge::PlannerOverrideContext, error::GraphError, Graph},
    planner::{best::find_best_combination, fetch::fetch_graph::FetchGraph},
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
        override_context: PlannerOverrideContext,
    ) -> Result<QueryPlan, PlannerError> {
        let best_paths_per_leaf = walk_operation(
            &self.graph,
            &self.supergraph,
            &override_context,
            normalized_operation,
        )?;
        let query_tree = find_best_combination(&self.graph, best_paths_per_leaf).unwrap();
        let mut fetch_graph = build_fetch_graph_from_query_tree(
            &self.graph,
            &self.supergraph,
            &override_context,
            query_tree,
        )?;
        add_variables_to_fetch_steps(&mut fetch_graph, &normalized_operation.variable_definitions)?;
        let query_plan = build_query_plan_from_fetch_graph(fetch_graph, &self.supergraph)?;

        Ok(query_plan)
    }
}

pub fn add_variables_to_fetch_steps(
    graph: &mut FetchGraph,
    variables: &Option<Vec<VariableDefinition>>,
) -> Result<(), PlannerError> {
    if let Some(variables) = variables {
        let mut nodes_to_patch: Vec<(NodeIndex, Vec<VariableDefinition>)> = Vec::new();

        for (node_index, node_weight) in graph.all_nodes() {
            if let Some(usage) = &node_weight.variable_usages {
                let relevant_variables = usage
                    .iter()
                    .filter_map(|used_var_name| {
                        variables
                            .iter()
                            .find(|op_var| op_var.name == *used_var_name)
                    })
                    .cloned()
                    .collect::<Vec<VariableDefinition>>();

                nodes_to_patch.push((node_index, relevant_variables));
            }
        }

        for (node_index, relevant_variables) in nodes_to_patch {
            let step = graph.get_step_data_mut(node_index)?;
            step.variable_definitions = Some(relevant_variables);
        }
    }

    Ok(())
}
