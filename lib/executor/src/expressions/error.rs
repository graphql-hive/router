use vrl::prelude::ExpressionError;

/// Errors that can occur during VRL expression compilation
#[derive(Debug, thiserror::Error, Clone)]
#[error("Failed to compile VRL expression '{expression}': {diagnostics}")]
pub struct ExpressionCompileError {
    pub expression: String,
    pub diagnostics: String,
}

impl ExpressionCompileError {
    pub fn new(expression: String, diagnostics: String) -> Self {
        Self {
            expression,
            diagnostics,
        }
    }
}

/// Errors that can occur during VRL expression execution
#[derive(Debug, thiserror::Error, Clone)]
#[error("Failed to execute VRL expression: {0}")]
pub struct ExpressionExecutionError(#[from] pub ExpressionError);

/// Errors that can occur during program resolution
#[derive(Debug, thiserror::Error)]
pub enum ProgramResolutionError<T: std::error::Error> {
    #[error("Failed to execute expression: {0}")]
    ExecutionFailed(#[source] ExpressionExecutionError),

    #[error("Failed to convert result: {0}")]
    ConversionFailed(#[source] T),
}
