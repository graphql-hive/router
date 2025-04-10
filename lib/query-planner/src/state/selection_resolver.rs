use super::subgraph_state::SubgraphState;

pub struct SelectionResolver {
    subgraph_state: SubgraphState,
}

impl SelectionResolver {
    pub fn new_from_state(subgraph: SubgraphState) -> Self {
        Self {
            subgraph_state: subgraph,
        }
    }
}
