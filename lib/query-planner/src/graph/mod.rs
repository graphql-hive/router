pub mod edge;
pub mod node;
mod tests;

use std::{
    collections::HashMap,
    fmt::{Debug, Display},
};

use graphql_parser_hive_fork::{
    query::{Selection, SelectionSet},
    schema::ParseError,
};
use graphql_tools::ast::{SchemaDocumentExtension, TypeExtension};
use node::SubgraphType;
use petgraph::{
    dot::Dot,
    graph::{EdgeIndex, Edges, NodeIndex},
    Directed, Direction, Graph as Petgraph,
};
use thiserror::Error;

use crate::{
    federation_spec::FederationRules,
    supergraph_metadata::{RootType, SupergraphDefinition, SupergraphState},
};

use super::graph::{edge::Edge, node::Node};

type Graph = Petgraph<Node, Edge, Directed>;

#[derive(Debug, Default)]
pub struct GraphQLSatisfiabilityGraph {
    pub graph: Graph,
    pub query_root: NodeIndex,
    pub mutation_root: Option<NodeIndex>,
    pub subscription_root: Option<NodeIndex>,
    pub node_to_index: HashMap<String, NodeIndex>,
}

#[derive(Debug, Error)]
pub enum GraphQLSatisfiabilityGraphError {
    #[error("failed to parse schema: {0}")]
    ParseSchemaError(#[from] ParseError),
    #[error("failed to locate 'from' node with id {0}")]
    FromEdgeIdNotFound(String),
    #[error("failed to locate 'to' node with id {0}")]
    ToEdgeIdNotFound(String),
}

impl GraphQLSatisfiabilityGraph {
    pub fn new_from_supergraph(
        supergraph_ir: &SupergraphState,
    ) -> Result<Self, GraphQLSatisfiabilityGraphError> {
        let mut instance = GraphQLSatisfiabilityGraph {
            node_to_index: HashMap::new(),
            graph: Graph::new(),
            ..Default::default()
        };

        instance.build_graph(supergraph_ir)?;

        Ok(instance)
    }

    pub fn node(&self, node_index: NodeIndex) -> &Node {
        self.graph.node_weight(node_index).unwrap()
    }

    pub fn edge(&self, edge_id: EdgeIndex) -> &Edge {
        self.graph.edge_weight(edge_id).unwrap()
    }

    fn build_graph(
        &mut self,
        state: &SupergraphState,
    ) -> Result<(), GraphQLSatisfiabilityGraphError> {
        self.build_root_nodes(state);
        self.link_root_edges(state);
        self.build_field_edges(state);
        self.build_interface_implementation_edges(state)?;
        self.build_entity_reference_edges(state)?;
        self.build_viewed_field_edges(state)?;

        Ok(())
    }

    fn build_root_nodes(&mut self, state: &SupergraphState<'_>) {
        self.query_root =
            self.upsert_node(Node::QueryRoot(state.document.query_type().name.clone()));
        self.mutation_root = state
            .document
            .mutation_type()
            .map(|mutation_type| self.upsert_node(Node::MutationRoot(mutation_type.name.clone())));
        self.subscription_root = state.document.subscription_type().map(|subscription_type| {
            self.upsert_node(Node::SubscriptionRoot(subscription_type.name.clone()))
        });
    }

    pub fn upsert_node(&mut self, node: Node) -> NodeIndex {
        let id = node.id();

        if self.node_to_index.contains_key(&id) {
            return *self.node_to_index.get(&id).unwrap();
        }

        let index = self.graph.add_node(node);
        self.node_to_index.insert(id, index);
        index
    }

    pub fn upsert_edge(&mut self, head: NodeIndex, tail: NodeIndex, edge: Edge) -> EdgeIndex {
        let existing_edge = self.graph.edge_indices().find(|edge_index| {
            let (source, target) = self.graph.edge_endpoints(*edge_index).unwrap();
            let edge_weight = self.graph.edge_weight(*edge_index).unwrap();
            let is_same_nodes = source == head && target == tail;
            let is_same_edge = edge_weight == &edge;

            is_same_nodes && is_same_edge
        });

        if let Some(edge) = existing_edge {
            edge
        } else {
            self.graph.add_edge(head, tail, edge)
        }
    }

    fn build_entity_reference_edges(
        &mut self,
        state: &SupergraphState<'_>,
    ) -> Result<(), GraphQLSatisfiabilityGraphError> {
        for (def_name, definition) in state.definitions.iter() {
            for join_type1 in definition.join_types() {
                for join_type2 in definition.join_types() {
                    let head =
                        self.upsert_node(Node::subgraph_type(def_name, &join_type1.graph_id));

                    if join_type1.graph_id != join_type2.graph_id {
                        if let Some(key) = &join_type2.key {
                            let tail = self
                                .upsert_node(Node::subgraph_type(def_name, &join_type2.graph_id));

                            self.upsert_edge(head, tail, Edge::EntityMove(key.clone()));
                        }
                    } else if let Some(key) = &join_type1.key {
                        self.upsert_edge(head, head, Edge::EntityMove(key.clone()));
                    }
                }
            }
        }

        Ok(())
    }

    fn build_interface_implementation_edges(
        &mut self,
        state: &SupergraphState<'_>,
    ) -> Result<(), GraphQLSatisfiabilityGraphError> {
        for (def_name, definition) in state
            .definitions
            .iter()
            .filter(|(_, d)| matches!(d, SupergraphDefinition::Object(_)))
        {
            for join_implements in definition.join_implements() {
                let tail =
                    self.upsert_node(Node::subgraph_type(def_name, &join_implements.graph_id));
                let head = self.upsert_node(Node::subgraph_type(
                    &join_implements.interface,
                    &join_implements.graph_id,
                ));

                self.upsert_edge(
                    head,
                    tail,
                    Edge::AbstractMove(definition.name().to_string()),
                );
            }
        }

        Ok(())
    }

    pub fn find_definition_node(
        &self,
        definition_name: &str,
        subgraph: &str,
    ) -> Option<(NodeIndex, &Node)> {
        let id = Node::id_from(definition_name, Some(subgraph));

        self.node_to_index
            .get(&id)
            .map(|&index| (index, &self.graph[index]))
    }

    pub fn root_query_node(&self) -> &Node {
        &self.graph[self.query_root]
    }

    pub fn root_mutation_node(&self) -> Option<&Node> {
        if let Some(mutation_root) = self.mutation_root {
            Some(&self.graph[mutation_root])
        } else {
            None
        }
    }

    pub fn root_subscription_node(&self) -> Option<&Node> {
        if let Some(subscription_root) = self.subscription_root {
            Some(&self.graph[subscription_root])
        } else {
            None
        }
    }

    pub fn edges_to(&self, node_index: NodeIndex) -> Edges<'_, Edge, Directed> {
        self.graph.edges_directed(node_index, Direction::Incoming)
    }

    pub fn edges_from(&self, node_index: NodeIndex) -> Edges<'_, Edge, Directed> {
        self.graph.edges_directed(node_index, Direction::Outgoing)
    }

    fn link_root_edges(&mut self, state: &SupergraphState<'_>) {
        for (def_name, definition) in state.definitions.iter() {
            if let Some(root_type) = definition.try_into_root_type() {
                for graph_id in definition.subgraphs().iter() {
                    for (field_name, field_definition) in definition.fields() {
                        let (is_available, _) = FederationRules::check_field_subgraph_availability(
                            field_definition,
                            graph_id,
                            definition,
                        );

                        if !is_available {
                            continue;
                        }

                        let head = match root_type {
                            RootType::Query => self.query_root,
                            RootType::Mutation => self.mutation_root.unwrap(),
                            RootType::Subscription => self.subscription_root.unwrap(),
                        };

                        let tail = self.upsert_node(Node::subgraph_type(def_name, graph_id));

                        self.upsert_edge(
                            head,
                            tail,
                            Edge::RootEntrypoint {
                                field_name: field_name.clone(),
                            },
                        );
                    }
                }
            }
        }
    }

    fn build_field_edges(&mut self, state: &SupergraphState<'_>) {
        for (def_name, definition) in state.definitions.iter() {
            for graph_id in definition.subgraphs().iter() {
                if definition.is_defined_in_subgraph(graph_id) {
                    for (field_name, field_definition) in definition.fields().iter() {
                        let (is_available, maybe_join_field) =
                            FederationRules::check_field_subgraph_availability(
                                field_definition,
                                graph_id,
                                definition,
                            );

                        let target_type = field_definition.source.field_type.inner_type();

                        match (is_available, maybe_join_field) {
                            (true, Some(join_field)) => {
                                let is_external = join_field.external.is_some_and(|v| v)
                                    && join_field.requires.is_none();
                                let has_provides = join_field.provides.is_some();

                                if is_external || has_provides {
                                    continue;
                                }

                                let head =
                                    self.upsert_node(Node::subgraph_type(def_name, graph_id));
                                let tail =
                                    self.upsert_node(Node::subgraph_type(target_type, graph_id));

                                self.upsert_edge(
                                    head,
                                    tail,
                                    Edge::create_field_move(
                                        field_name.clone(),
                                        Some(join_field.clone()),
                                    ),
                                );
                            }
                            (true, None) => {
                                let head =
                                    self.upsert_node(Node::subgraph_type(def_name, graph_id));
                                let tail =
                                    self.upsert_node(Node::subgraph_type(target_type, graph_id));
                                self.upsert_edge(
                                    head,
                                    tail,
                                    Edge::create_field_move(field_name.clone(), None),
                                );
                            }
                            // The field is not available in the current subgraph
                            _ => {}
                        };
                    }
                }
            }
        }
    }

    fn handle_viewed_selection_set(
        &mut self,
        state: &SupergraphState,
        selection_set: &SelectionSet<'static, String>,
        graph_id: &str,
        parent_type_def: &SupergraphDefinition<'_>,
        head: NodeIndex,
        view_id: u64,
    ) {
        for selection in selection_set.items.iter() {
            match selection {
                Selection::Field(field) => {
                    let is_leaf = field.selection_set.items.is_empty();
                    let field_in_parent = parent_type_def.fields().get(&field.name).unwrap();
                    let return_type_name = field_in_parent.source.field_type.inner_type();

                    let subgraph_type = SubgraphType {
                        name: return_type_name.to_string(),
                        subgraph: graph_id.to_string(),
                    };
                    let tail = self.upsert_node(match is_leaf {
                        true => Node::SubgraphType(subgraph_type),
                        false => Node::SubgraphTypeView {
                            view_id,
                            node: subgraph_type,
                            selection_set: field.selection_set.to_string(),
                        },
                    });

                    self.upsert_edge(
                        head,
                        tail,
                        Edge::create_field_move(field.name.to_string(), None),
                    );

                    if !is_leaf {
                        let return_type = state.definitions.get(return_type_name).unwrap();

                        self.handle_viewed_selection_set(
                            state,
                            &field.selection_set,
                            graph_id,
                            return_type,
                            tail,
                            view_id,
                        );
                    }
                }
                _ => unimplemented!("fragments are not supported in provides yet"),
            };
        }
    }

    fn build_viewed_field_edges(
        &mut self,
        state: &SupergraphState,
    ) -> Result<(), GraphQLSatisfiabilityGraphError> {
        for (_, definition) in state.definitions.iter() {
            for join_type in definition.join_types().iter() {
                let mut view_id = 0;

                for (field_name, field_definition) in definition.fields().iter() {
                    for join_field in field_definition.join_field.iter() {
                        if join_field
                            .graph_id
                            .as_ref()
                            .is_some_and(|v| v == &join_type.graph_id)
                            && join_field.provides.is_some()
                        {
                            if let Some(selection_set) = FederationRules::parse_provides(join_field)
                            {
                                view_id += 1;

                                let head = self.upsert_node(Node::subgraph_type(
                                    definition.name(),
                                    &join_type.graph_id,
                                ));

                                let return_type_name =
                                    field_definition.source.field_type.inner_type();

                                let tail = self.upsert_node(Node::SubgraphTypeView {
                                    view_id,
                                    node: SubgraphType {
                                        name: return_type_name.to_string(),
                                        subgraph: join_type.graph_id.to_string(),
                                    },
                                    selection_set: selection_set.to_string(),
                                });

                                self.upsert_edge(
                                    head,
                                    tail,
                                    Edge::create_field_move(
                                        field_name.to_string(),
                                        Some(join_field.clone()),
                                    ),
                                );

                                let return_type = state.definitions.get(return_type_name).unwrap();

                                self.handle_viewed_selection_set(
                                    state,
                                    &selection_set,
                                    &join_type.graph_id,
                                    return_type,
                                    tail,
                                    view_id,
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Print me with `println!("{}", graph);` to see the graph in DOT/digraph format.
impl Display for GraphQLSatisfiabilityGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", Dot::with_config(&self.graph, &[]))
    }
}
