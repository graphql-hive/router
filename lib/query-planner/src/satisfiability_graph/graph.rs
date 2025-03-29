use std::{
    collections::HashMap,
    fmt::{Debug, Display},
};

use graphql_parser_hive_fork::schema::ParseError;
use graphql_tools::ast::{SchemaDocumentExtension, TypeExtension};
use petgraph::{
    dot::Dot,
    graph::{EdgeIndex, Edges, NodeIndex},
    visit::EdgeRef,
    Directed, Direction, Graph as Petgraph,
};
use thiserror::Error;

use crate::supergraph_metadata::{RootType, SupergraphDefinition, SupergraphMetadata};

use super::{edge::Edge, node::Node};

type Graph = Petgraph<Node, Edge, Directed>;
type RootEntrypointsMap = HashMap<(OperationType, String), NodeIndex>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationType {
    Query,
    Mutation,
    Subscription,
}

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
    pub node_to_index: HashMap<String, NodeIndex>,
    /// Utility functions for working with root entrypoints.
    /// This struct has key of: (OperationType, RootFieldName)
    /// It helps us to decide where to start the graph traversal for a given root selection set field.
    ///
    /// Example:
    ///    (Query, allProducts) -> SubgraphType { name: "Query", subgraph: "PRODUCT" }
    pub root_entrypoints: RootEntrypointsMap,
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

    fn build_root_entrypoints(lookup_table: &mut LookupTable) {
        fn build_from_root_node(index: NodeIndex, table: &mut LookupTable, op_type: OperationType) {
            let query_edges = table.graph.edges_directed(index, Direction::Outgoing);

            for edge_ref in query_edges {
                let edge = edge_ref.weight();
                if let Edge::Root { field_name } = edge {
                    table
                        .root_entrypoints
                        .insert((op_type, field_name.clone()), edge_ref.target());
                }
            }
        }

        build_from_root_node(lookup_table.query_root, lookup_table, OperationType::Query);

        if let Some(mutation_root) = lookup_table.mutation_root {
            build_from_root_node(mutation_root, lookup_table, OperationType::Mutation);
        }

        if let Some(subscription_root) = lookup_table.subscription_root {
            build_from_root_node(subscription_root, lookup_table, OperationType::Subscription);
        }
    }

    pub fn node(&self, node_index: NodeIndex) -> &Node {
        self.lookup.graph.node_weight(node_index).unwrap()
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

        Self::build_root_entrypoints(&mut self.lookup);

        Ok(())
    }

    fn build_nodes_for_definition(lookup: &mut LookupTable, definition: &SupergraphDefinition) {
        for join_type in definition.join_types() {
            // We can skip creating nodes for types that are not available in the current subgraph,
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

            if let Some(root_type) = definition.root_type() {
                // Check if this subgraph defines any fields on this root type
                let fields_in_subgraph = definition
                    .fields()
                    .values()
                    .filter(|field| {
                        if field.join_field.is_empty() {
                            // For fields without join_field, check if the parent type is in this subgraph
                            definition.available_in_subgraph(&join_type.graph)
                        } else {
                            // For fields with join_field, check if any specify this subgraph
                            field.join_field.iter().any(|jf| {
                                jf.graph
                                    .as_ref()
                                    .map(|g| g == &join_type.graph)
                                    .unwrap_or(false)
                            })
                        }
                    })
                    .collect::<Vec<_>>();

                // Only connect if this subgraph defines fields on this root type
                if !fields_in_subgraph.is_empty() {
                    let from = match root_type {
                        RootType::Query => lookup.query_root,
                        RootType::Mutation => lookup.mutation_root.unwrap(),
                        RootType::Subscription => lookup.subscription_root.unwrap(),
                    };

                    for field in fields_in_subgraph {
                        lookup.link_nodes(
                            from,
                            node_index,
                            Edge::Root {
                                field_name: field.source.name.to_string(),
                            },
                        );
                    }
                }
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
            // Current code: Creates edge FROM implementing type TO interface
            let id_from = Node::id_from(definition.name(), Some(&join_implements.graph));
            let id_to = Node::id_from(&join_implements.interface, Some(&join_implements.graph));

            lookup.link_nodes_using_indices(
                &id_from,
                &id_to,
                Edge::InterfaceImplementation(join_implements.interface.clone()),
            )?;

            lookup.link_nodes_using_indices(
                &id_to,   // From interface
                &id_from, // To implementing type
                Edge::InterfaceImplementation(definition.name().to_string()),
            )?;
        }

        Ok(())
    }

    pub fn find_definition_node(
        &self,
        definition_name: &str,
        subgraph: &str,
    ) -> Option<(NodeIndex, &Node)> {
        let id = Node::id_from(definition_name, Some(subgraph));

        self.lookup
            .node_to_index
            .get(&id)
            .map(|&index| (index, &self.lookup.graph[index]))
    }

    pub fn root_query_node(&self) -> &Node {
        &self.lookup.graph[self.lookup.query_root]
    }

    pub fn root_mutation_node(&self) -> Option<&Node> {
        if let Some(mutation_root) = self.lookup.mutation_root {
            Some(&self.lookup.graph[mutation_root])
        } else {
            None
        }
    }

    pub fn root_subscription_node(&self) -> Option<&Node> {
        if let Some(subscription_root) = self.lookup.subscription_root {
            Some(&self.lookup.graph[subscription_root])
        } else {
            None
        }
    }

    pub fn edges_to(&self, node_index: NodeIndex) -> Edges<'_, Edge, Directed> {
        self.lookup
            .graph
            .edges_directed(node_index, Direction::Incoming)
    }

    pub fn edges_from(&self, node_index: NodeIndex) -> Edges<'_, Edge, Directed> {
        self.lookup
            .graph
            .edges_directed(node_index, Direction::Outgoing)
    }

    pub fn debug_node_index(&self, node_index: NodeIndex) {
        let node = &self.lookup.graph[node_index];
        println!("Node {:?}: {:?}", node_index, node);
    }

    pub fn debug_edges_from(&self, node_index: NodeIndex) {
        self.debug_node_index(node_index);
        let edges = self.edges_from(node_index);

        for edge in edges {
            let target = edge.target();
            let edge_data = edge.weight();
            println!("   Edge {:?} to {:?}", edge_data, self.lookup.graph[target]);
        }
    }

    fn build_field_edges(
        lookup: &mut LookupTable,
        definition: &SupergraphDefinition,
    ) -> Result<(), GraphQLSatisfiabilityGraphError> {
        for join_type in definition.join_types() {
            for (name, field) in definition.fields().iter() {
                let id_from = Node::id_from(definition.name(), Some(&join_type.graph));
                let type_to = field.source.field_type.inner_type();

                // Check if this field belongs to the current subgraph
                let field_belongs_to_subgraph = if field.join_field.is_empty() {
                    // If there's no join_field, the field might be a scalar or a local type
                    // We should check if the parent type is available in this subgraph
                    definition.available_in_subgraph(&join_type.graph)
                } else {
                    // For fields with join_field directives, check if any of them specify this subgraph
                    field.join_field.iter().any(|jf| {
                        let has_override = jf
                            .override_value
                            .as_ref()
                            .map(|override_value| {
                                // TODO: override is defines as String, it's better to
                                // do some actual checking of the enum value here.
                                *override_value == join_type.graph.to_lowercase()
                            })
                            .unwrap_or(false);

                        has_override
                            || jf
                                .graph
                                .as_ref()
                                .map(|g| g == &join_type.graph)
                                .unwrap_or(false)
                    })
                };

                if field_belongs_to_subgraph {
                    // First, create the edge in the current subgraph
                    let id_to = Node::id_from(type_to, Some(&join_type.graph));
                    // Find the specific join_field for this subgraph if it exists
                    let subgraph_join_field = field
                        .join_field
                        .iter()
                        .find(|jf| {
                            jf.graph
                                .as_ref()
                                .map(|g| g == &join_type.graph)
                                .unwrap_or(false)
                        })
                        .cloned();

                    lookup.link_nodes_using_indices(
                        &id_from,
                        &id_to,
                        Edge::field(name.to_string(), subgraph_join_field),
                    )?;

                    // Only create cross-subgraph edges for fields returning entity or interface types
                    // Check if the type exists in other subgraphs (indicating it's a federated type)
                    let is_federated_type = definition.join_types().len() > 1;

                    // For root fields or fields that return federated types
                    if definition.is_root() || is_federated_type {
                        // Count how many subgraphs this type exists in
                        let mut subgraph_count = 0;
                        for check_join_type in definition.join_types() {
                            let check_id = Node::id_from(type_to, Some(&check_join_type.graph));
                            if lookup.node_to_index.contains_key(&check_id) {
                                subgraph_count += 1;
                            }
                        }

                        // Only proceed if this type exists in multiple subgraphs (true federation)
                        if subgraph_count > 1 {
                            for other_join_type in definition.join_types() {
                                // Skip the current subgraph as we already created that edge
                                if other_join_type.graph == join_type.graph {
                                    continue;
                                }

                                // Check if the type exists in this other subgraph
                                let other_id_to =
                                    Node::id_from(type_to, Some(&other_join_type.graph));
                                if lookup.node_to_index.contains_key(&other_id_to) {
                                    // Create an edge to this type in the other subgraph
                                    lookup.link_nodes_using_indices(
                                        &id_from,
                                        &other_id_to,
                                        Edge::field(name.to_string(), None),
                                    )?;
                                }
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
        write!(f, "{:?}", Dot::with_config(&self.lookup.graph, &[]))
    }
}
