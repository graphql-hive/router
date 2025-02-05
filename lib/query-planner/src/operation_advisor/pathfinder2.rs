use graphql_parser_hive_fork::query::{OperationDefinition, SelectionSet};
use petgraph::graph::NodeIndex;
use thiserror::Error;

use crate::satisfiability_graph::graph::GraphQLSatisfiabilityGraph;

pub struct Pathfinder2<'a> {
    graph: &'a GraphQLSatisfiabilityGraph,
}

pub struct PathToField {
    field_name: String,
    subgraph: String,
}

#[derive(Debug, Error)]
pub enum PathfinderError {
    #[error("failed to find graph entrypoint")]
    NoEntrypoint,
}

impl<'a> Pathfinder2<'a> {
    pub fn new(graph: &'a GraphQLSatisfiabilityGraph) -> Self {
        Self { graph }
    }

    pub fn traverse_operation(
        &self,
        operation: &OperationDefinition<'static, String>,
    ) -> Result<(), PathfinderError> {
        let (graph_entrypoint, selection_set) = match &operation {
            OperationDefinition::Query(query) => {
                (Some(self.graph.lookup.query_root), &query.selection_set)
            }
            OperationDefinition::SelectionSet(selection_set) => {
                (Some(self.graph.lookup.query_root), selection_set)
            }
            OperationDefinition::Mutation(mutation) => {
                (self.graph.lookup.mutation_root, &mutation.selection_set)
            }
            OperationDefinition::Subscription(subscription) => (
                self.graph.lookup.subscription_root,
                &subscription.selection_set,
            ),
        };

        match graph_entrypoint {
            None => Err(PathfinderError::NoEntrypoint),
            Some(entrypoint) => self.traverse_selection_set(entrypoint, selection_set),
        }
    }

    fn traverse_selection_set(
        &self,
        graph_entrypoint: NodeIndex,
        selection_set: &SelectionSet<'static, String>,
    ) -> Result<(), PathfinderError> {
        Ok(())
    }
}
