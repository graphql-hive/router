use graphql_parser_hive_fork::query::{OperationDefinition, SelectionSet};
use graphql_tools::static_graphql::query::Selection;
use petgraph::graph::NodeIndex;

use crate::satisfiability_graph::graph::GraphQLSatisfiabilityGraph;

pub struct Pathfinder<'a> {
    graph: &'a GraphQLSatisfiabilityGraph,
}

impl<'a> Pathfinder<'a> {
    pub fn new(graph: &'a GraphQLSatisfiabilityGraph) -> Self {
        Pathfinder { graph }
    }

    pub fn traverse_operation(
        &self,
        operation: &OperationDefinition<'static, String>,
    ) -> Vec<DependencyNode> {
        match operation {
            OperationDefinition::Query(query) => {
                self.traverse_selection_set(self.graph.lookup.query_root, &query.selection_set)
            }
            OperationDefinition::SelectionSet(selection_set) => {
                self.traverse_selection_set(self.graph.lookup.query_root, selection_set)
            }
            OperationDefinition::Mutation(mutation) => self.traverse_selection_set(
                self.graph.lookup.mutation_root.unwrap(),
                &mutation.selection_set,
            ),
            OperationDefinition::Subscription(subscription) => self.traverse_selection_set(
                self.graph.lookup.subscription_root.unwrap(),
                &subscription.selection_set,
            ),
        }
    }

    pub fn traverse_selection_set(
        &self,
        entrypoint: NodeIndex,
        selection_set: &SelectionSet<'static, String>,
    ) -> Vec<DependencyNode> {
        selection_set
            .items
            .iter()
            .map(|item| match item {
                Selection::Field(field) => {
                    let mut node = DependencyNode::new(field.name.clone());

                    if !field.selection_set.items.is_empty() {
                        node.children =
                            self.traverse_selection_set(entrypoint, &field.selection_set);
                    }

                    node
                }
                _ => todo!("not implemented"),
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct DependencyNode {
    pub field_name: String,
    pub children: Vec<DependencyNode>,
}

impl DependencyNode {
    pub fn new(field_name: String) -> Self {
        DependencyNode {
            field_name,
            children: Vec::new(),
        }
    }
}
