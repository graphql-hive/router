use std::{
    collections::HashMap,
    fmt::{Debug, Display},
};

use graphql_parser_hive_fork::schema::ParseError;
use graphql_tools::ast::{SchemaDocumentExtension, TypeExtension};
use petgraph::{
    dot::Dot,
    graph::{EdgeIndex, NodeIndex},
    Directed, Graph as Petgraph,
};
use thiserror::Error;

use crate::{
    edge::Edge,
    node::Node,
    supergraph::{RootType, SupergraphDefinition, SupergraphMetadata},
};

type Graph = Petgraph<Node, Edge, Directed>;

#[derive(Debug)]
pub struct GraphQLSatisfiabilityGraph {
    pub lookup: LookupTable,
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

#[derive(Debug, Default)]
pub struct LookupTable {
    pub graph: Graph,
    pub query_root: NodeIndex,
    pub mutation_root: Option<NodeIndex>,
    pub subscription_root: Option<NodeIndex>,
    node_to_index: HashMap<String, NodeIndex>,
}

impl LookupTable {
    pub fn create_node_for_type(&mut self, node: Node) -> NodeIndex {
        let id = node.id();

        if self.node_to_index.contains_key(&id) {
            return *self.node_to_index.get(&id).unwrap();
        }

        let index = self.graph.add_node(node);
        self.node_to_index.insert(id, index);
        index
    }

    pub fn link_nodes_using_indices(
        &mut self,
        from_id: &str,
        to_id: &str,
        edge: Edge,
    ) -> Result<EdgeIndex, GraphQLSatisfiabilityGraphError> {
        let from = self.node_to_index.get(from_id).ok_or(
            GraphQLSatisfiabilityGraphError::FromEdgeIdNotFound(from_id.to_string()),
        )?;

        let to = self.node_to_index.get(to_id).ok_or(
            GraphQLSatisfiabilityGraphError::ToEdgeIdNotFound(from_id.to_string()),
        )?;

        Ok(self.link_nodes(*from, *to, edge))
    }

    pub fn link_nodes(&mut self, from: NodeIndex, to: NodeIndex, new_edge: Edge) -> EdgeIndex {
        let existing_edge = self.graph.edge_indices().find(|edge_index| {
            let (source, target) = self.graph.edge_endpoints(*edge_index).unwrap();
            let edge_weight = self.graph.edge_weight(*edge_index).unwrap();
            let is_same_nodes = source == from && target == to;
            let is_same_edge = edge_weight == &new_edge;

            is_same_nodes && is_same_edge
        });

        if let Some(edge) = existing_edge {
            edge
        } else {
            self.graph.add_edge(from, to, new_edge)
        }
    }
}

impl GraphQLSatisfiabilityGraph {
    pub fn new_from_supergraph(
        supergraph_ir: &SupergraphMetadata,
    ) -> Result<Self, GraphQLSatisfiabilityGraphError> {
        let mut lookup = LookupTable {
            node_to_index: HashMap::new(),
            graph: Graph::new(),
            ..Default::default()
        };

        lookup.query_root = lookup.create_node_for_type(Node::QueryRoot(
            supergraph_ir.document.query_type().name.clone(),
        ));
        lookup.mutation_root = supergraph_ir.document.mutation_type().map(|mutation_type| {
            lookup.create_node_for_type(Node::MutationRoot(mutation_type.name.clone()))
        });
        lookup.subscription_root =
            supergraph_ir
                .document
                .subscription_type()
                .map(|subscription_type| {
                    lookup.create_node_for_type(Node::SubscriptionRoot(
                        subscription_type.name.clone(),
                    ))
                });

        let mut instance = GraphQLSatisfiabilityGraph { lookup };

        instance.build_graph(supergraph_ir)?;

        Ok(instance)
    }

    fn build_graph(
        &mut self,
        supergraph_ir: &SupergraphMetadata,
    ) -> Result<(), GraphQLSatisfiabilityGraphError> {
        // First, we iterate all the definitions in the supergraph and build nodes for each of them.
        // We create all the nodes first, and then we create the edges.
        for definition in supergraph_ir.definitions.values() {
            Self::build_nodes_for_definition(&mut self.lookup, definition);
        }

        // Then, build the edges between the nodes.
        for definition in supergraph_ir.definitions.values() {
            // @join__type
            // First, we iterate the fields and find references across subgraphs
            // These needs to be linked with EntityReference edges, and basically represent the relation
            // between the same type in different subgraphs.
            Self::build_entity_reference_edges(&mut self.lookup, definition)?;
            // @join__field
            // Then, we iterate the fields and create edges between the fields and the field types.
            // These fields are simpler and considered local
            Self::build_field_edges(&mut self.lookup, definition)?;
            // @join__implements
            // Then, we iterate and build the edges that are based on interfaces.
            Self::build_interface_edges(&mut self.lookup, definition)?;
        }

        Ok(())
    }

    fn build_nodes_for_definition(lookup: &mut LookupTable, definition: &SupergraphDefinition) {
        for join_type in definition.join_types() {
            // We can skip creating Nodes for types that are not available in the current subgraph,
            // when iterating the root type.
            // We determine that by checking the fields of the object type and check what fields are involved in a join.
            if !definition.available_in_subgraph(&join_type.graph) && definition.is_root() {
                continue;
            }

            let node = Node::SubgraphType {
                name: definition.name().to_string(),
                subgraph: join_type.graph.to_string(),
            };

            // Create the Node for the type, and link it to the root if it's a root type.
            let node_index = lookup.create_node_for_type(node);

            match definition.root_type() {
                Some(RootType::Query) => {
                    lookup.link_nodes(lookup.query_root, node_index, Edge::Root);
                }
                Some(RootType::Mutation) => {
                    lookup.link_nodes(lookup.mutation_root.unwrap(), node_index, Edge::Root);
                }
                Some(RootType::Subscription) => {
                    lookup.link_nodes(lookup.subscription_root.unwrap(), node_index, Edge::Root);
                }
                _ => {}
            }

            // Iterate the fields of the object type and create nodes for them.
            // This will ensure smooth edges creation process, if all nodes already exists.
            for field in definition.fields().values() {
                let field_type = field.source.field_type.inner_type();

                // If a field does not have join field, it means that the field is local,
                // and we can create a node for it.
                if field.join_field.is_empty() {
                    let node = Node::SubgraphType {
                        name: field_type.to_string(),
                        subgraph: join_type.graph.to_string(),
                    };

                    lookup.create_node_for_type(node);
                } else {
                    // If a field has join field, we need to create a node for each subgraph it's available in.
                    for jf in &field.join_field {
                        if let Some(subgraph) = &jf.graph {
                            let node = Node::SubgraphType {
                                name: field_type.to_string(),
                                subgraph: subgraph.to_string(),
                            };

                            lookup.create_node_for_type(node);
                        }
                    }
                }
            }
        }
    }

    fn build_entity_reference_edges(
        lookup: &mut LookupTable,
        definition: &SupergraphDefinition,
    ) -> Result<(), GraphQLSatisfiabilityGraphError> {
        for join_type1 in definition.join_types() {
            for join_type2 in definition.join_types() {
                if join_type1.graph != join_type2.graph {
                    let name = definition.name();
                    let id_from = Node::id_from(name, Some(&join_type1.graph));
                    let id_to = Node::id_from(name, Some(&join_type2.graph));

                    if let Some(key) = &join_type2.key {
                        lookup.link_nodes_using_indices(
                            &id_from,
                            &id_to,
                            Edge::EntityReference(key.clone()),
                        )?;
                    }
                }
            }
        }

        Ok(())
    }

    fn build_interface_edges(
        lookup: &mut LookupTable,
        definition: &SupergraphDefinition,
    ) -> Result<(), GraphQLSatisfiabilityGraphError> {
        for join_implements in definition.join_implements() {
            let id_from = Node::id_from(definition.name(), Some(&join_implements.graph));
            let id_to = Node::id_from(&join_implements.interface, Some(&join_implements.graph));

            lookup.link_nodes_using_indices(
                &id_from,
                &id_to,
                Edge::InterfaceImplementation(join_implements.interface.clone()),
            )?;
        }

        Ok(())
    }

    fn build_field_edges(
        lookup: &mut LookupTable,
        definition: &SupergraphDefinition,
    ) -> Result<(), GraphQLSatisfiabilityGraphError> {
        for join_type in definition.join_types() {
            for (name, field) in definition.fields().iter() {
                let id_from = Node::id_from(definition.name(), Some(&join_type.graph));
                let type_to = field.source.field_type.inner_type();

                // If the field has no join field, we can link it directly to the field type.
                if field.join_field.is_empty() {
                    let id_to = Node::id_from(type_to, Some(&join_type.graph));

                    lookup.link_nodes_using_indices(
                        &id_from,
                        &id_to,
                        Edge::Field {
                            name: name.to_string(),
                            join_field: None,
                        },
                    )?;
                } else if definition.available_in_subgraph(&join_type.graph) {
                    for jf in &field.join_field {
                        if let Some(join_field_subgraph) = &jf.graph {
                            let subgraph_target = if *join_field_subgraph != join_type.graph {
                                join_field_subgraph
                            } else {
                                &join_type.graph
                            };

                            let id_to = Node::id_from(type_to, Some(subgraph_target));
                            lookup.link_nodes_using_indices(
                                &id_from,
                                &id_to,
                                Edge::Field {
                                    name: name.to_string(),
                                    join_field: Some(jf.clone()),
                                },
                            )?;
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
        write!(f, "{:?}", Dot::with_config(&self.lookup.graph, &[]))
    }
}
