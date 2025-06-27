use petgraph::graph::NodeIndex;

use crate::{graph::error::GraphError, state::supergraph_state::OperationKind};

#[derive(Debug, thiserror::Error)]
pub enum WalkOperationError {
    #[error("Root type of {0} not found")]
    MissingRootType(OperationKind),
    #[error("Graph error: {0}")]
    GraphFailure(Box<GraphError>),
    #[error("Tail node missing info")]
    TailMissingInfo(NodeIndex),
    #[error("Type Definition of '{0}' not found in Supergraph")]
    TypeNotFound(String),
    #[error("No paths found for selection item: {0}")]
    NoPathsFound(String),
}

impl From<GraphError> for WalkOperationError {
    fn from(error: GraphError) -> Self {
        WalkOperationError::GraphFailure(Box::new(error))
    }
}
