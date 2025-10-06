use crate::{headers::errors::HeaderRuleRuntimeError, projection::error::ProjectionError};

#[derive(thiserror::Error, Debug, Clone)]
pub enum PlanExecutionError {
    #[error("Projection faiure: {0}")]
    ProjectionFailure(#[from] ProjectionError),
    #[error(transparent)]
    HeaderPropagation(#[from] HeaderRuleRuntimeError),
}
