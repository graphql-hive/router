use std::collections::HashSet;

use petgraph::graph::EdgeIndex;

use crate::ast::type_aware_selection::TypeAwareSelection;

// TODO: Consider interior mutability with Rc<RefCell<ExcludedFromLookup>> to avoid full clone while traversing
pub struct ExcludedFromLookup {
    pub graph_ids: HashSet<String>,
    pub requirement: HashSet<TypeAwareSelection>,
    pub edge_indices: Vec<EdgeIndex>,
}

impl Default for ExcludedFromLookup {
    fn default() -> Self {
        Self {
            graph_ids: HashSet::new(),
            requirement: HashSet::new(),
            edge_indices: Vec::new(),
        }
    }
}

impl ExcludedFromLookup {
    pub fn new() -> ExcludedFromLookup {
        Default::default()
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
        requirements: &HashSet<TypeAwareSelection>,
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
