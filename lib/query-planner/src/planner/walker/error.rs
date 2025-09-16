use petgraph::graph::NodeIndex;

use crate::{
    graph::error::GraphError, state::supergraph_state::OperationKind,
    utils::cancellation::CancellationError,
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum WalkOperationError {
    #[error("Root type of {0} not found")]
    MissingRootType(OperationKind),
    #[error("Graph error: {0}")]
    GraphFailure(Box<GraphError>),
    #[error("Tail node missing info")]
    TailMissingInfo(NodeIndex),
    #[error("Type Definition of '{0}' not found in Supergraph")]
    TypeNotFound(String),
    #[error("Field '{0}' not found in type '{1}'")]
    FieldNotFound(String, String),
    #[error("No paths found for selection item: {0}")]
    NoPathsFound(String),
    #[error(transparent)]
    CancellationError(#[from] CancellationError),
    /// In case of a shareable field resolving an interface, all object types implementing the interface
    /// must resolve the field in the same way.
    ///
    /// If one of the fields (defined by the interface) is @external in one of the object types,
    /// it means that the Query Planner would have to decide which subgraph to pick from to resolve the field
    /// of each individual object type.
    /// This would result in more than one request being made to the subgraphs.
    ///
    /// See: https://github.com/graphql-hive/federation-gateway-audit/blob/514fec87122d561a4f7b12a66a91a6a35b1a76a7/src/test-suites/corrupted-supergraph-node-id/test.ts#L6-L11
    #[error("The shareable field '{field_name}' on interface '{type_name}' is not resolvable by all of its object types in all subgraphs, which violates the '@shareable' contract.")]
    InconsistentShareableField {
        field_name: String,
        type_name: String,
    },
}

impl From<GraphError> for WalkOperationError {
    fn from(error: GraphError) -> Self {
        WalkOperationError::GraphFailure(Box::new(error))
    }
}
