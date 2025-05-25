use petgraph::graph::NodeIndex;

use crate::{
    ast::selection_item::SelectionItem, graph::error::GraphError,
    state::supergraph_state::OperationKind,
};

#[derive(Debug, thiserror::Error)]
pub enum WalkOperationError {
    #[error("Root type of {0} not found")]
    MissingRootType(OperationKind),
    #[error("Graph error: {0}")]
    GraphFailure(GraphError),
    #[error("Tail node missing info")]
    TailMissingInfo(NodeIndex),
    #[error("No paths found for selection item: {0}")]
    NoPathsFound(SelectionItem),
}

impl From<GraphError> for WalkOperationError {
    fn from(error: GraphError) -> Self {
        WalkOperationError::GraphFailure(error)
    }
}
