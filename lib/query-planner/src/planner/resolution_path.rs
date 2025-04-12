use std::fmt::Debug;

use petgraph::graph::{EdgeIndex, NodeIndex};

use crate::graph::Graph;

#[derive(Clone)]
pub struct ResolutionPath {
    pub root_node: NodeIndex,
    pub edges: Vec<EdgeIndex>,
    pub required_paths_for_edges: Vec<Vec<ResolutionPath>>,
    pub cost: u64,
}

impl Debug for ResolutionPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = f.debug_struct("");
        let mut out = out.field("cost", &self.cost);

        if self.edges.is_empty() {
            out = out.field("empty", &true).field("head", &self.root_node);
        } else {
            out = out.field(
                "egdes",
                &self
                    .edges
                    .iter()
                    .map(|i| format!("{:?}", i))
                    .collect::<Vec<String>>()
                    .join(" --> "),
            );
        }
        out.finish()
    }
}

impl ResolutionPath {
    pub fn new(root_node: NodeIndex) -> Self {
        ResolutionPath {
            root_node,
            edges: vec![],
            required_paths_for_edges: vec![],
            cost: 0,
        }
    }

    pub fn tail(&self, graph: &Graph) -> NodeIndex {
        if self.edges.is_empty() {
            self.root_node
        } else {
            let last_edge_id = self.edges.last().unwrap();
            graph.get_edge_tail(last_edge_id)
        }
    }

    pub fn advance_to(&self, graph: &Graph, edge_index: &EdgeIndex) -> ResolutionPath {
        let edge = graph.edge(*edge_index);
        let mut new_edges = self.edges.clone();
        new_edges.push(*edge_index);

        ResolutionPath {
            root_node: self.root_node,
            edges: new_edges,
            required_paths_for_edges: self.required_paths_for_edges.clone(),
            cost: self.cost + edge.cost(),
        }
    }
}
