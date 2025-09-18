use crate::{graph::error::GraphError, utils::cancellation::CancellationError};

use super::fetch::error::FetchGraphError;

#[derive(Debug, Clone, thiserror::Error)]
pub enum QueryPlanError {
    #[error("FetchGraph error: {0}")]
    FetchGraphFailure(Box<FetchGraphError>),
    #[error("Graph error: {0}")]
    GraphFailure(Box<GraphError>),
    #[error("Root fetch is missing")]
    NoRoot,
    #[error("Failed to build a plan")]
    EmptyPlan,
    #[error("Query planning finished with unresolved pending steps")]
    UnexpectedPendingState,
    #[error("Internal Error: {0}")]
    Internal(String),
    #[error(transparent)]
    CancellationError(#[from] CancellationError),
}

impl From<FetchGraphError> for QueryPlanError {
    fn from(error: FetchGraphError) -> Self {
        QueryPlanError::FetchGraphFailure(Box::new(error))
    }
}

impl From<GraphError> for QueryPlanError {
    fn from(error: GraphError) -> Self {
        QueryPlanError::GraphFailure(Box::new(error))
    }
}
