use std::sync::Arc;

use error::QueryPlanError;
use fetch::{error::FetchGraphError, fetch_graph::build_fetch_graph_from_query_tree};
use graphql_tools::parser::schema;
use petgraph::graph::NodeIndex;
use plan_nodes::QueryPlan;
use query_plan::build_query_plan_from_fetch_graph;
use walker::{error::WalkOperationError, walk_operation};

use crate::{
    ast::operation::{OperationDefinition, VariableDefinition},
    consumer_schema::ConsumerSchema,
    graph::{edge::PlannerOverrideContext, error::GraphError, Graph},
    planner::{
        best::find_best_combination,
        fetch::{fetch_graph::FetchGraph, state::MultiTypeFetchStep},
    },
    state::supergraph_state::{OperationKind, SupergraphState},
    utils::cancellation::{CancellationError, CancellationToken},
};

pub mod best;
mod error;
pub mod fetch;
pub mod plan_nodes;
pub mod query_plan;
pub mod tree;
pub mod walker;

#[derive(Debug, Clone, Default)]
pub struct QueryPlannerOptions {
    pub experimental_abstract_type_folding: bool,
}

pub struct Planner<'a> {
    graph: Graph<'a>,
    pub supergraph: &'a SupergraphState,
    pub consumer_schema: Arc<ConsumerSchema>,
    options: QueryPlannerOptions,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PlannerError {
    #[error("failed to initalize relations graph: {0}")]
    GraphInitError(#[from] GraphError),
    #[error("failed to locate operation to execute")]
    MissingOperationToExecute,
    #[error("walker failed to locate path: {0}")]
    PathLocatorError(#[from] WalkOperationError),
    #[error("failed to build fetch graph: {0}")]
    FailedToConstructFetchGraph(Box<FetchGraphError>),
    #[error("failed to build plan: {0}")]
    QueryPlanBuildFailed(#[from] QueryPlanError),
    #[error(transparent)]
    CancellationError(#[from] CancellationError),
}

impl Planner<'static> {
    pub fn new_from_supergraph(
        parsed_supergraph: &schema::Document<'static, String>,
        options: QueryPlannerOptions,
    ) -> Result<Self, PlannerError> {
        let supergraph_ref: &'static SupergraphState =
            Box::leak(Box::new(SupergraphState::new(parsed_supergraph)));
        let graph = Graph::graph_from_supergraph_state(supergraph_ref)?;
        let consumer_schema = Arc::new(ConsumerSchema::new_from_supergraph(parsed_supergraph));

        Ok(Planner {
            graph,
            supergraph: supergraph_ref,
            consumer_schema,
            options,
        })
    }
}

impl<'a> Planner<'a> {
    pub fn new_from_supergraph_state(
        supergraph_state: &'a SupergraphState,
        parsed_supergraph: &schema::Document<'static, String>,
        options: QueryPlannerOptions,
    ) -> Result<Self, PlannerError> {
        let graph = Graph::graph_from_supergraph_state(supergraph_state)?;
        let consumer_schema = Arc::new(ConsumerSchema::new_from_supergraph(parsed_supergraph));

        Ok(Planner {
            graph,
            supergraph: supergraph_state,
            consumer_schema,
            options,
        })
    }

    #[inline]
    pub fn plan_from_normalized_operation(
        &self,
        normalized_operation: &OperationDefinition,
        override_context: PlannerOverrideContext,
        cancellation_token: &CancellationToken,
    ) -> Result<QueryPlan, PlannerError> {
        let best_paths_per_leaf = walk_operation(
            &self.graph,
            self.supergraph,
            &override_context,
            normalized_operation,
            cancellation_token,
        )?;
        let query_tree =
            find_best_combination(&self.graph, best_paths_per_leaf, cancellation_token)?;
        let mut fetch_graph = build_fetch_graph_from_query_tree(
            &self.graph,
            self.supergraph,
            &override_context,
            query_tree,
            normalized_operation
                .operation_kind
                .clone()
                .unwrap_or(OperationKind::Query),
            &self.options,
            cancellation_token,
        )
        .map_err(|e| PlannerError::FailedToConstructFetchGraph(Box::new(e)))?;
        add_variables_to_fetch_steps(&mut fetch_graph, &normalized_operation.variable_definitions)?;
        let query_plan =
            build_query_plan_from_fetch_graph(fetch_graph, self.supergraph, cancellation_token)?;

        Ok(query_plan)
    }
}

pub fn add_variables_to_fetch_steps<'a>(
    graph: &mut FetchGraph<'a, MultiTypeFetchStep>,
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
            let step = graph
                .get_step_data_mut(node_index)
                .map_err(|e| PlannerError::FailedToConstructFetchGraph(Box::new(e)))?;
            step.variable_definitions = Some(relevant_variables);
        }
    }

    Ok(())
}
