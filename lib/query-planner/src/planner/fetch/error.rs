use crate::graph::{error::GraphError, node::Node};

#[derive(Debug, thiserror::Error)]
pub enum FetchGraphError {
    #[error("Graph error: {0}")]
    GraphFailure(GraphError),
    #[error("Missing FetchStep: {0}")]
    MissingStep(usize),
    #[error("Expected an index, got None")]
    IndexNone,
    #[error("Subgraph name: {0}")]
    MissingSubgraphName(Node),
    #[error("Expected to have one root step, but found: {0}")]
    NonSingleRootStep(usize),
}

impl From<GraphError> for FetchGraphError {
    fn from(error: GraphError) -> Self {
        FetchGraphError::GraphFailure(error)
    }
}
