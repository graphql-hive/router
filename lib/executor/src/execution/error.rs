use crate::projection::error::ProjectionError;

#[derive(thiserror::Error, Debug, Clone)]
pub enum PlanExecutionError {
    #[error("Projection faiure: {0}")]
    ProjectionFailure(ProjectionError),
}

impl From<ProjectionError> for PlanExecutionError {
    fn from(error: ProjectionError) -> Self {
        PlanExecutionError::ProjectionFailure(error)
    }
}
