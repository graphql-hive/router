use std::{collections::HashMap, fmt::Display};

mod edge;
mod node;

use edge::DiscoveryEdge;
use graphql_parser_hive_fork::schema::{Definition, Document, TypeDefinition};
use node::DiscoveryNode;
use petgraph::{dot::Dot, graph::NodeIndex, Directed, Graph as Petgraph};

use crate::federation_spec::FederationSpec;

type Graph = Petgraph<DiscoveryNode, DiscoveryEdge, Directed>;

pub struct DiscoveryGraph {
    lookup: DiscoveryGraphLookup,
}

pub struct DiscoveryGraphLookup {
    pub graph: Graph,
    node_to_index: HashMap<String, NodeIndex>,
}

impl DiscoveryGraph {
    pub fn new_from_supergraph_metadata(supergraph: &Document<'static, String>) -> Self {
        let (graph, map) = Self::build_graph(&supergraph);

        let r = DiscoveryGraph {
            lookup: DiscoveryGraphLookup {
                graph,
                node_to_index: map,
            },
        };

        println!("{}", r);

        r
    }

    fn add_node(
        graph: &mut Graph,
        map: &mut HashMap<String, NodeIndex>,
        node: DiscoveryNode,
    ) -> NodeIndex {
        let name = node.id();
        let index = graph.add_node(node);
        map.insert(name, index);
        index
    }

    fn build_graph(supergraph: &Document<'static, String>) -> (Graph, HashMap<String, NodeIndex>) {
        let mut map: HashMap<String, NodeIndex> = HashMap::new();
        let mut graph = Graph::new();

        // First, iterate and build all the relevant nodes
        for definition in supergraph
            .definitions
            .iter()
            .filter(|d| !FederationSpec::is_core_definition(d))
        {
            match definition {
                Definition::TypeDefinition(type_definition) => match type_definition {
                    TypeDefinition::Scalar(scalar_type) => {
                        Self::add_node(
                            &mut graph,
                            &mut map,
                            DiscoveryNode::ScalarType(scalar_type.name.to_string()),
                        );
                    }
                    TypeDefinition::Object(object_type) => {
                        Self::add_node(
                            &mut graph,
                            &mut map,
                            DiscoveryNode::ObjectType(object_type.name.to_string()),
                        );
                    }
                    TypeDefinition::Interface(interface_type) => {
                        Self::add_node(
                            &mut graph,
                            &mut map,
                            DiscoveryNode::InterfaceType(interface_type.name.to_string()),
                        );
                    }
                    TypeDefinition::Union(union_type) => {
                        Self::add_node(
                            &mut graph,
                            &mut map,
                            DiscoveryNode::UnionType(union_type.name.to_string()),
                        );
                    }
                    TypeDefinition::Enum(enum_type) => {
                        Self::add_node(
                            &mut graph,
                            &mut map,
                            DiscoveryNode::EnumType(enum_type.name.to_string()),
                        );
                    }
                    _ => {}
                },
                Definition::TypeExtension(_type_extension) => todo!(),
                _ => {}
            }
        }

        (graph, map)
    }
}

/// Print me with `println!("{}", graph);` to see the graph in DOT/digraph format.
impl Display for DiscoveryGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", Dot::with_config(&self.lookup.graph, &[]))
    }
}
