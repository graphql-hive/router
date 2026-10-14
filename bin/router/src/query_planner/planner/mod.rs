use std::{collections::HashMap, sync::Arc};

use error::QueryPlanError;
use fetch::{error::FetchGraphError, fetch_graph::build_fetch_graph_from_query_tree};
use graphql_tools::parser::schema;
use petgraph::graph::NodeIndex;
use plan_nodes::{Planning, QueryPlan};
use query_plan::build_query_plan_from_fetch_graph;
use walker::{error::WalkOperationError, walk_operation};

use crate::query_planner::{
    ast::operation::{OperationDefinition, VariableDefinition},
    consumer_schema::ConsumerSchema,
    graph::{edge::PlannerOverrideContext, error::GraphError, Graph},
    planner::{
        best::find_best_combination,
        fetch::{fetch_graph::FetchGraph, state::MultiTypeFetchStep},
    },
    state::supergraph_state::SupergraphState,
    utils::cancellation::{CancellationError, CancellationToken},
};

pub mod best;
mod error;
pub mod fetch;
pub mod plan_nodes;
pub mod query_plan;
pub mod tree;
pub mod walker;

#[derive(Debug, Clone)]
pub struct QueryPlannerOptions {
    pub experimental_abstract_type_folding: bool,
    /// The most aliased `_entities` fields one batched entity request to a subgraph may carry.
    /// Entity fetches that would be batched beyond this limit are split into several batches.
    ///
    /// Values in `subgraph_max_aliases_per_request` take precedence for their subgraph.
    pub max_aliases_per_request: usize,
    /// Per-subgraph overrides of `max_aliases_per_request`, keyed by subgraph name.
    pub subgraph_max_aliases_per_request: HashMap<String, usize>,
}

impl Default for QueryPlannerOptions {
    fn default() -> Self {
        Self {
            experimental_abstract_type_folding: false,
            max_aliases_per_request: 15,
            subgraph_max_aliases_per_request: HashMap::new(),
        }
    }
}

impl QueryPlannerOptions {
    /// Returns the alias limit for the given subgraph, never less than 1.
    pub fn max_aliases_per_request(&self, subgraph_name: &str) -> usize {
        self.subgraph_max_aliases_per_request
            .get(subgraph_name)
            .copied()
            .unwrap_or(self.max_aliases_per_request)
            .max(1)
    }
}

pub struct Planner {
    graph: Graph,
    pub supergraph: SupergraphState,
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
    FailedToConstructFetchGraph(#[from] FetchGraphError),
    #[error("failed to build plan: {0}")]
    QueryPlanBuildFailed(#[from] QueryPlanError),
    #[error(transparent)]
    CancellationError(#[from] CancellationError),
}

impl Planner {
    pub fn new_from_supergraph(
        parsed_supergraph: &schema::Document<'static, String>,
        options: QueryPlannerOptions,
    ) -> Result<Self, PlannerError> {
        let supergraph_state = SupergraphState::new(parsed_supergraph);
        Self::new_from_supergraph_state(supergraph_state, parsed_supergraph, options)
    }

    pub fn new_from_supergraph_state(
        supergraph_state: SupergraphState,
        parsed_supergraph: &schema::Document<'static, String>,
        options: QueryPlannerOptions,
    ) -> Result<Self, PlannerError> {
        let graph = Graph::graph_from_supergraph_state(&supergraph_state)?;
        let consumer_schema = ConsumerSchema::new_from_supergraph(parsed_supergraph);

        Ok(Planner {
            graph,
            consumer_schema: Arc::new(consumer_schema),
            supergraph: supergraph_state,
            options,
        })
    }

    #[inline]
    pub fn plan_from_normalized_operation(
        &self,
        normalized_operation: &OperationDefinition,
        override_context: PlannerOverrideContext,
        cancellation_token: &CancellationToken,
    ) -> Result<QueryPlan<Planning>, PlannerError> {
        let best_paths_per_leaf = walk_operation(
            &self.graph,
            &self.supergraph,
            &override_context,
            normalized_operation,
            cancellation_token,
        )?;
        let query_tree =
            find_best_combination(&self.graph, best_paths_per_leaf, cancellation_token)?;
        let mut fetch_graph = build_fetch_graph_from_query_tree(
            &self.graph,
            &self.supergraph,
            &override_context,
            query_tree,
            normalized_operation,
            &self.options,
            cancellation_token,
        )?;
        add_variables_to_fetch_steps(&mut fetch_graph, &normalized_operation.variable_definitions)?;
        let query_plan = build_query_plan_from_fetch_graph(
            fetch_graph,
            &self.supergraph,
            &self.options,
            cancellation_token,
        )?;

        Ok(query_plan)
    }
}

pub fn add_variables_to_fetch_steps(
    graph: &mut FetchGraph<MultiTypeFetchStep>,
    variables: &Option<Vec<VariableDefinition>>,
) -> Result<(), PlannerError> {
    let indices: Vec<NodeIndex> = graph.step_indices().collect();
    for node_index in indices {
        let step = graph.get_step_data_mut(node_index)?;
        if !step.context_variable_definitions.is_empty() {
            step.variable_usages.get_or_insert_default().extend(
                step.context_variable_definitions
                    .iter()
                    .map(|variable| variable.name.clone()),
            );
        }
        if let Some(usage) = &step.variable_usages {
            let mut relevant_variables = usage
                .iter()
                .filter_map(|used_var_name| {
                    variables
                        .iter()
                        .flatten()
                        .find(|op_var| op_var.name == *used_var_name)
                })
                .cloned()
                .collect::<Vec<_>>();
            relevant_variables.extend(step.context_variable_definitions.iter().cloned());
            if variables.is_some() || !relevant_variables.is_empty() {
                step.variable_definitions = Some(relevant_variables);
            }
        }
    }

    Ok(())
}
