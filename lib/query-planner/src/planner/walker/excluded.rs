use std::collections::HashSet;

use petgraph::graph::EdgeIndex;

use crate::graph::selection::Selection;

// TODO: Consider interior mutability with Rc<RefCell<ExcludedFromLookup>> to avoid full clone while traversing
pub struct ExcludedFromLookup {
    pub graph_ids: HashSet<String>,
    pub requirement: HashSet<Selection>,
    pub edge_indices: Vec<EdgeIndex>,
}

impl ExcludedFromLookup {
    pub fn new() -> ExcludedFromLookup {
        ExcludedFromLookup {
            graph_ids: HashSet::new(),
            requirement: HashSet::new(),
            edge_indices: Vec::new(),
        }
    }

    pub fn next_with_graph_id(&self, graph_id: &str) -> ExcludedFromLookup {
        let mut graph_ids = self.graph_ids.clone();
        graph_ids.insert(graph_id.to_string());

        ExcludedFromLookup {
            graph_ids,
            requirement: self.requirement.clone(),
            edge_indices: self.edge_indices.clone(),
        }
    }

    pub fn next(
        &self,
        graph_id: &str,
        requirements: &HashSet<Selection>,
        edges_indices: &[EdgeIndex],
    ) -> ExcludedFromLookup {
        let mut graph_ids = self.graph_ids.clone();
        graph_ids.insert(graph_id.to_string());

        ExcludedFromLookup {
            graph_ids,
            requirement: requirements.clone(),
            edge_indices: edges_indices.to_vec(),
        }
    }
}
