use super::supergraph_state::SupergraphState;

pub type SubgraphId = String;

pub struct SubgraphState {
    graph_id: SubgraphId,
    // type_map: HashMap<String>,
}

impl SubgraphState {
    pub fn decompose_from_supergraph(
        graph_id: &SubgraphId,
        supergraph_state: &SupergraphState,
    ) -> Self {
        Self {
            graph_id: graph_id.clone(),
            // type_map: HashMap::new(),
        }
    }
}
