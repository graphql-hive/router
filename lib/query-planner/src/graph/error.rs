use crate::state::{
    selection_resolver::SelectionResolverError,
    supergraph_state::{OperationKind, SupergraphStateError},
};
use petgraph::graph::{EdgeIndex, NodeIndex};

#[derive(thiserror::Error, Debug)]
pub enum GraphError {
    #[error("Node with index '{0:?}' was not found")]
    NodeNotFound(NodeIndex),
    #[error("Edge with index '{0:?}' was not found")]
    EdgeNotFound(EdgeIndex),
    #[error("Unexpected missing root type {0}")]
    MissingRootType(OperationKind),
    #[error("Definition with name '{0}' was not found")]
    DefinitionNotFound(String),
    #[error("Field named '{0}' was not found in definition name '{1}'")]
    FieldDefinitionNotFound(String, String),
    #[error("Supergraph state error: {0}")]
    SupergraphStateError(Box<SupergraphStateError>),
    #[error("Selection resolver error: {0}")]
    SelectionResolverError(Box<SelectionResolverError>),
}

impl From<SupergraphStateError> for GraphError {
    fn from(err: SupergraphStateError) -> Self {
        GraphError::SupergraphStateError(Box::new(err))
    }
}

impl From<SelectionResolverError> for GraphError {
    fn from(err: SelectionResolverError) -> Self {
        GraphError::SelectionResolverError(Box::new(err))
    }
}
