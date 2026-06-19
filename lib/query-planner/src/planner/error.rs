use crate::{
    ast::fragment_expansion::FragmentExpansionError, graph::error::GraphError,
    utils::cancellation::CancellationError,
};

use super::fetch::error::FetchGraphError;

#[derive(Debug, Clone, thiserror::Error)]
pub enum QueryPlanError {
    #[error("FetchGraph error: {0}")]
    FetchGraphFailure(String),
    #[error("Graph error: {0}")]
    GraphFailure(#[from] GraphError),
    #[error("Root fetch is missing")]
    NoRoot,
    #[error("Failed to build a plan")]
    EmptyPlan,
    #[error("Query planning finished with unresolved pending steps")]
    UnexpectedPendingState,
    #[error("Internal Error: {0}")]
    Internal(String),
    #[error("Fragment expansion error: {0}")]
    FragmentExpansionFailure(#[from] FragmentExpansionError),
    #[error(transparent)]
    CancellationError(#[from] CancellationError),
}

impl From<FetchGraphError> for QueryPlanError {
    fn from(err: FetchGraphError) -> Self {
        QueryPlanError::FetchGraphFailure(err.to_string())
    }
}
