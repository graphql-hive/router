use super::fetch::error::FetchGraphError;

#[derive(Debug, thiserror::Error)]
pub enum QueryPlanError {
    #[error("FetchGraph error: {0}")]
    FetchGraphFailure(Box<FetchGraphError>),
    #[error("Root fetch is missing")]
    NoRoot,
    #[error("Failed to build a plan")]
    EmptyPlan,
    #[error("Query planning finished with unresolved pending steps")]
    UnexpectedPendingState,
    #[error("Internal Error: {0}")]
    Internal(String),
}

impl From<FetchGraphError> for QueryPlanError {
    fn from(error: FetchGraphError) -> Self {
        QueryPlanError::FetchGraphFailure(Box::new(error))
    }
}
