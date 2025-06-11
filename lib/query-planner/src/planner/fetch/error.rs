use crate::{
    graph::{error::GraphError, node::Node},
    planner::walker::error::WalkOperationError,
};

#[derive(Debug, thiserror::Error)]
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
    #[error("Expected a single parent")]
    NonSingleParent,
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
}

impl From<GraphError> for FetchGraphError {
    fn from(error: GraphError) -> Self {
        FetchGraphError::GraphFailure(Box::new(error))
    }
}
