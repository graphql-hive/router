use crate::state::supergraph_state::RootOperationType;
use petgraph::graph::{EdgeIndex, NodeIndex};

#[derive(thiserror::Error, Debug)]
pub enum GraphError {
    #[error("Node with index '{0:?}' was not found")]
    NodeNotFound(NodeIndex),
    #[error("Edge with index '{0:?}' was not found")]
    EdgeNotFound(EdgeIndex),
    #[error("Unexpected missing root type {0}")]
    MissingRootType(RootOperationType),
    #[error("Definition with name '{0}' was not found")]
    DefinitionNotFound(String),
    #[error("Field named '{0}' was not found in definition name '{1}'")]
    FieldDefinitionNotFound(String, String),
}
