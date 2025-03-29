use std::collections::HashMap;

use graphql_parser_hive_fork::query::{Field, OperationDefinition, Selection, SelectionSet};
use petgraph::graph::{EdgeIndex, NodeIndex};

use crate::satisfiability_graph::graph::{GraphQLSatisfiabilityGraph, OperationType};

pub struct OperationTraversal<'a, 'b> {
    operation: &'a OperationDefinition<'static, String>,
    graph: &'b GraphQLSatisfiabilityGraph,
}

type PossibleRoutesMap = HashMap<String, Vec<TraversalJump>>;

#[derive(Debug)]
pub struct TraversalNode {
    pub field_name: String,
    pub children: PossibleRoutesMap,
}

#[derive(Debug)]
enum TraversalJump {
    Direct {
        through_edge: EdgeIndex,
        children: PossibleRoutesMap,
    },
    Indirect {
        through_edge: EdgeIndex,
        children: PossibleRoutesMap,
    },
}

impl<'a, 'b> OperationTraversal<'a, 'b> {
    pub fn new(
        operation: &'a OperationDefinition<'static, String>,
        graph: &'b GraphQLSatisfiabilityGraph,
    ) -> Self {
        OperationTraversal { operation, graph }
    }

    pub fn travel_graph(&self) -> Vec<TraversalNode> {
        let all_possible_routes = match self.operation {
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
        };

        println!("all_possible_routes: {:#?}", all_possible_routes);

        todo!()
    }

    fn traverse_selection_set(
        &self,
        parent_node: NodeIndex,
        selection_set: &SelectionSet<'static, String>,
    ) -> PossibleRoutesMap {
        selection_set
            .items
            .iter()
            .map(|item| match item {
                Selection::Field(field) => (
                    field.name.to_string(),
                    self.process_field(parent_node, field),
                ),
                _ => todo!("not implemented"),
            })
            .collect()
    }

    fn traverse_root_selection_set(
        &self,
        op_type: OperationType,
        selection_set: &SelectionSet<'static, String>,
    ) -> PossibleRoutesMap {
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

                    println!(
                        "[traverse_root_selection_set] field_name: {}, root: {:?}",
                        field_name, root
                    );

                    (
                        field_name.to_string(),
                        self.process_field(root.clone(), field),
                    )
                }
                _ => todo!("not implemented"),
            })
            .collect()
    }

    fn process_field(
        &self,
        parent_node: NodeIndex,
        field: &Field<'static, String>,
    ) -> Vec<TraversalJump> {
        println!("  processing field '{}'", field.name);
        let possible_routes = self.graph.find_possible_routes(parent_node, &field.name);

        for (edge_id, target_node_index) in possible_routes {
            println!(
                "       found possible route via {:?},  to {:?}",
                edge_id, target_node_index
            );
            let edge = self.graph.edge(edge_id);
            // TraversalJump::from(edge);
            // let mut node = TraversalNode::new(field.name.clone());

            // if !field.selection_set.items.is_empty() {
            //     node.children = self.traverse_selection_set(jump., &field.selection_set);
            // }
            // println!("  Possible route: {:?}", jump);
        }

        vec![]
    }
}
