use crate::{
    ast::type_aware_selection::TypeAwareSelectionError,
    graph::{error::GraphError, node::Node},
    planner::walker::error::WalkOperationError,
    utils::cancellation::CancellationError,
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum FetchGraphError {
    #[error("Internal Error: {0}")]
    Internal(String),
    #[error("Graph error: {0}")]
    GraphFailure(Box<GraphError>),
    #[error("Missing FetchStep: {0} {1}")]
    MissingStep(usize, String),
    #[error("Missing parent of FetchStep: {0}")]
    MissingParent(usize),
    #[error("Expected an index, got None")]
    IndexNone,
    #[error("Expected a single parent, but the FetchStep [{0}] has many")]
    NonSingleParent(usize),
    #[error("Subgraph name: {0}")]
    MissingSubgraphName(Box<Node>),
    #[error("Missing requirement tree for @requires")]
    MissingRequirement,
    #[error("Expected a single children of the @requires query tree node")]
    ManyChildrenOfRequirement,
    #[error("Expected to have one root step, but found: {0}")]
    NonSingleRootStep(usize),
    #[error("Expected different indexes: {0}")]
    SameNodeIndex(usize),
    #[error("Failed ot find satisfiable key for @requires: {0}")]
    SatisfiableKeyFailure(Box<WalkOperationError>),
    #[error("Expected a FetchStep with Mutation to have its order defined")]
    MutationStepWithNoOrder,
    #[error("Index mapping got lost")]
    IndexMappingLost,
    #[error("Expected Fetch Steps not to be empty")]
    EmptyFetchSteps,
    #[error("Unexpected case where two user-defined fields are conflicting!")]
    UnexpectedConflict,
    #[error("Input types are equal but response_path are different!")]
    MismatchedResponsePath,
    #[error("Expected {0}")]
    UnexpectedEdgeMove(String),
    #[error("Expected a subgraph type")]
    ExpectedSubgraphType,
    #[error("Expected @requires")]
    MissingRequires,
    #[error(transparent)]
    SelectionSetManipulationError(#[from] TypeAwareSelectionError),
    #[error(transparent)]
    CancellationError(#[from] CancellationError),
}

impl From<GraphError> for FetchGraphError {
    fn from(error: GraphError) -> Self {
        FetchGraphError::GraphFailure(Box::new(error))
    }
}
