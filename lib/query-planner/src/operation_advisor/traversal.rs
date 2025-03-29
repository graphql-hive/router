use std::collections::HashMap;

use graphql_parser_hive_fork::query::{Field, OperationDefinition, Selection, SelectionSet};
use petgraph::graph::{EdgeIndex, NodeIndex};

use crate::satisfiability_graph::edge::Edge;
use crate::satisfiability_graph::graph::{GraphQLSatisfiabilityGraph, OperationType};

pub struct OperationTraversal<'a, 'b> {
    operation: &'a OperationDefinition<'static, String>,
    graph: &'b GraphQLSatisfiabilityGraph,
}

#[derive(Debug)]
struct PossibleRoutesMap {
    pub map: HashMap<String, Vec<TraversalJump>>,
}

impl PossibleRoutesMap {
    pub fn from_hashmap(map: HashMap<String, Vec<TraversalJump>>) -> Self {
        Self { map }
    }
}

#[derive(Debug)]
pub struct TraversalNode {
    pub field_name: String,
    pub children: Option<PossibleRoutesMap>,
}

#[derive(Debug)]
enum TraversalJump {
    Direct {
        through_edge: EdgeIndex,
        to_node: NodeIndex,
        children: Option<PossibleRoutesMap>,
    },
    Indirect {
        through_edge: EdgeIndex,
        to_node: NodeIndex,
        children: Option<PossibleRoutesMap>,
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
        PossibleRoutesMap::from_hashmap(
            selection_set
                .items
                .iter()
                .filter_map(|item| match item {
                    Selection::Field(field) => {
                        let process_field = self.process_field(parent_node, field);

                        process_field.map(|v| (field.name.to_string(), v))
                    }
                    _ => todo!("not implemented"),
                })
                .collect(),
        )
    }

    fn traverse_root_selection_set(
        &self,
        op_type: OperationType,
        selection_set: &SelectionSet<'static, String>,
    ) -> PossibleRoutesMap {
        PossibleRoutesMap::from_hashmap(
            selection_set
                .items
                .iter()
                .filter_map(|item| match item {
                    Selection::Field(field) => {
                        let field_name = field.name.as_str();
                        let root = self
                            .graph
                            .lookup
                            .root_entrypoints
                            .get(&(op_type, field_name.into()))
                            .unwrap();

                        let available_field_jumps = self.process_field(root.clone(), field);

                        available_field_jumps.map(|v| (field_name.to_string(), v))
                    }
                    _ => todo!("not implemented"),
                })
                .collect(),
        )
    }

    fn process_field(
        &self,
        parent_node: NodeIndex,
        field: &Field<'static, String>,
    ) -> Option<Vec<TraversalJump>> {
        println!("  processing field '{}'", field.name);
        let direct_paths = self
            .graph
            .find_possible_direct_routes(parent_node, &field.name);

        if !direct_paths.is_empty() {
            let valid_direct_paths = direct_paths
                .iter()
                .filter_map(
                    |(edge_id, target_node_index)| match self.graph.edge(*edge_id) {
                        Edge::Field { .. } => {
                            let children = if !field.selection_set.items.is_empty() {
                                let child_map = self.traverse_selection_set(
                                    *target_node_index,
                                    &field.selection_set,
                                );

                                // Check if child map is empty (dead end)
                                if child_map.map.is_empty() {
                                    None
                                } else {
                                    Some(child_map)
                                }
                            } else {
                                None
                            };

                            Some(TraversalJump::Direct {
                                through_edge: *edge_id,
                                to_node: *target_node_index,
                                children,
                            })
                        }
                        Edge::InterfaceImplementation(_name) => Some(TraversalJump::Indirect {
                            through_edge: *edge_id,
                            to_node: *target_node_index,
                            children: None,
                        }),
                        Edge::EntityReference(_v) => Some(TraversalJump::Indirect {
                            through_edge: *edge_id,
                            to_node: *target_node_index,
                            children: None,
                        }),
                        _ => None,
                    },
                )
                .collect::<Vec<_>>();

            if !valid_direct_paths.is_empty() {
                return Some(valid_direct_paths);
            }
        }

        None
    }
}
