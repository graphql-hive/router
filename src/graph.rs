use std::{collections::HashMap, fmt::Display};

use crate::supergraph::{SuperObjectTypeDefinition, SuperTypeDefinition, SupergraphIR};

pub struct Graph {
    supergraph: SupergraphIR,
    graph_name: String,
    // We do it for two reasons:
    // 1. We want to be able to quickly find all nodes/edges of a given type
    // 2. We want to avoid array length limit
    nodes_by_type_index: Vec<Vec<Node>>,
    // We have two indexes of edges:
    // 1. By head type
    // 2. By tail type
    // We do it to quickly pick edges by head/tail type, without iterating over all edges.
    edges_by_head_type_index: Vec<Vec<Edge>>,
    edges_by_tail_type_index: Vec<Vec<Edge>>,
    // To quickly find all nodes of a given type
    type_name_to_node_indexes: HashMap<String, Vec<i32>>,
}

impl Graph {
    pub fn new(supergraph: SupergraphIR, graph_name: String) -> Graph {
        Graph {
            supergraph,
            graph_name,
            nodes_by_type_index: Vec::new(),
            edges_by_head_type_index: Vec::new(),
            edges_by_tail_type_index: Vec::new(),
            type_name_to_node_indexes: HashMap::new(),
        }
    }

    pub fn add_from_roots(&mut self) {
        // find Query
        self.supergraph
            .type_definitions
            .get("Query")
            .and_then(|type_def| Some(self.create_nodes_and_edges_for_type(type_def)));
    }

    fn create_nodes_and_edges_for_type(&mut self, type_def: &SuperTypeDefinition) {
        match type_def {
            SuperTypeDefinition::Object(object_type) => {
                self.create_nodes_and_edges_for_object_type(object_type);
            }
        }
    }

    fn create_nodes_and_edges_for_object_type(
        &mut self,
        object_type: &SuperObjectTypeDefinition,
    ) -> &Node {
        let node = self.ensure_none_or_single_node(&object_type.name);

        if node.is_some() {
            return node.unwrap();
        }

        let head = self.createTypeNode(&object_type.name);

        // for field in object_type.fields.iter() {
        //     // self.createEdgeForObjectTypeField(head, field);
        // }

        return head;
    }

    fn ensure_none_or_single_node(&mut self, type_name: &str) -> Option<&Node> {
        let indexes = self.type_name_to_node_indexes.get(type_name);

        if let Some(indexes) = indexes {
            if indexes.len() > 1 {
                panic!("Expected only one node for {}", type_name);
            }

            // this.nodesByTypeIndex[indexes[0]][0]
            let first_index = indexes.first();

            if first_index.is_none() {
                panic!("Expected at least one node for {}", type_name);
            }

            let at = first_index.unwrap();
            return self
                .nodes_by_type_index
                .get(*at as usize)
                .and_then(|inner| inner.first());
        }

        None
    }

    fn createTypeNode(&mut self, type_name: &str) -> &Node {
        let node = self.type_name_to_node_indexes.get(type_name);
        if node.is_some() {
            panic!("Node for {} already exists in graph", type_name);
        }

        self.create_node(type_name)
    }

    fn create_node(&mut self, type_name: &str) -> &Node {
        self.nodes_by_type_index.push(Vec::new());
        let index = self.nodes_by_type_index.len() - 1;

        let node = Node {
            index: index as i32,
            is_leaf: false,
            type_name: type_name.to_string(),
            graph_name: self.graph_name.clone(),
        };

        self.nodes_by_type_index[index].push(node);
        self.edges_by_head_type_index.push(Vec::new());
        self.edges_by_tail_type_index.push(Vec::new());

        let node_indexes = self
            .type_name_to_node_indexes
            .entry(type_name.to_string())
            .or_insert(Vec::new());
        node_indexes.push(index as i32);

        self.nodes_by_type_index[index].last().unwrap()
    }
}

pub struct Node {
    index: i32,
    is_leaf: bool,
    type_name: String,
    graph_name: String,
}

pub struct Edge {
    pub head: Node,
    pub movement: Movement,
    pub tail: Node,
}

pub enum Movement {
    Field(FieldMovement),
}

pub struct FieldMovement {
    pub type_name: String,
    pub field_name: String,
}

impl Display for Movement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Movement::Field(field_movement) => write!(f, "{}", field_movement),
        }
    }
}

impl Display for FieldMovement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.field_name)
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.type_name, self.graph_name)
    }
}

impl Display for Edge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} -> {} -> {}", self.head, self.movement, self.tail)
    }
}
