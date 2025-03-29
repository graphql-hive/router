use graphql_parser_hive_fork::query::{Field, OperationDefinition, Selection, SelectionSet};
use petgraph::graph::NodeIndex;

use crate::satisfiability_graph::graph::{GraphQLSatisfiabilityGraph, OperationType};

pub struct OperationTraversal<'a, 'b> {
    operation: &'a OperationDefinition<'static, String>,
    graph: &'b GraphQLSatisfiabilityGraph,
}

#[derive(Debug)]
pub struct TraversalNode {
    pub field_name: String,
    pub children: Vec<TraversalNode>,
}

impl TraversalNode {
    pub fn new(field_name: String) -> Self {
        TraversalNode {
            field_name,
            children: Vec::new(),
        }
    }
}

impl<'a, 'b> OperationTraversal<'a, 'b> {
    pub fn new(
        operation: &'a OperationDefinition<'static, String>,
        graph: &'b GraphQLSatisfiabilityGraph,
    ) -> Self {
        OperationTraversal { operation, graph }
    }

    pub fn travel_graph(&self) -> Vec<TraversalNode> {
        match self.operation {
            OperationDefinition::Query(query) => {
                self.traverse_root_selection_set(OperationType::Query, &query.selection_set)
            }
            OperationDefinition::SelectionSet(selection_set) => {
                self.traverse_root_selection_set(OperationType::Query, &selection_set)
            }
            OperationDefinition::Mutation(mutation) => {
                self.traverse_root_selection_set(OperationType::Mutation, &mutation.selection_set)
            }
            OperationDefinition::Subscription(subscription) => self.traverse_root_selection_set(
                OperationType::Subscription,
                &subscription.selection_set,
            ),
        }
    }

    fn traverse_selection_set(
        &self,
        parent_node: NodeIndex,
        selection_set: &SelectionSet<'static, String>,
    ) -> Vec<TraversalNode> {
        selection_set
            .items
            .iter()
            .map(|item| match item {
                Selection::Field(field) => self.process_field(parent_node, field),
                _ => todo!("not implemented"),
            })
            .collect()
    }

    fn traverse_root_selection_set(
        &self,
        op_type: OperationType,
        selection_set: &SelectionSet<'static, String>,
    ) -> Vec<TraversalNode> {
        selection_set
            .items
            .iter()
            .map(|item| match item {
                Selection::Field(field) => {
                    let field_name = field.name.as_str();
                    let root = self
                        .graph
                        .lookup
                        .root_entrypoints
                        .get(&(op_type, field_name.into()))
                        .unwrap();

                    self.process_field(root.clone(), field)
                }
                _ => todo!("not implemented"),
            })
            .collect()
    }

    fn process_field(
        &self,
        parent_node: NodeIndex,
        field: &Field<'static, String>,
    ) -> TraversalNode {
        let mut node = TraversalNode::new(field.name.clone());

        if !field.selection_set.items.is_empty() {
            node.children = self.traverse_selection_set(parent_node, &field.selection_set);
        }

        node
    }
}
