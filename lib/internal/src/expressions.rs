use std::time::Duration;

// Re-export VRL under our internal namespace
pub use vrl;

pub use hive_console_sdk::expressions::error::{
    ExpressionCompileError, ExpressionExecutionError, ProgramResolutionError,
};
pub use hive_console_sdk::expressions::lib::{
    CompileExpression, ExecutableProgram, FromVrlValue, ToVrlValue,
};
pub use hive_console_sdk::expressions::values::duration::DurationConversionError;
pub use hive_console_sdk::expressions::values::header_value::HeaderValueConversionError;
pub use hive_console_sdk::expressions::values::string::StringConversionError;
use vrl::{compiler::Program as VrlProgram, core::Value as VrlValue};

/// Generic enum for a value that can be either static or computed via VRL expression
#[derive(Clone)]
pub enum ValueOrProgram<T> {
    /// A statically-known value
    Value(T),
    /// A VRL program that computes the value at runtime
    Program(Box<VrlProgram>),
}

impl<T> ValueOrProgram<T>
where
    T: FromVrlValue + Clone,
{
    /// Resolve this ValueOrProgram to a concrete value
    ///
    /// If this is a static value, returns it immediately.
    /// If this is a program, executes it against the provided context and converts the result.
    ///
    /// - `vrl_context_fn` - A function that returns the VRL value context for expression execution
    #[inline]
    pub fn resolve<F>(&self, vrl_context_fn: F) -> Result<T, ProgramResolutionError<T::Error>>
    where
        F: FnOnce() -> VrlValue,
    {
        match self {
            ValueOrProgram::Value(v) => Ok(v.clone()),
            ValueOrProgram::Program(vrl_program) => {
                let vrl_context = vrl_context_fn();
                let result_value = vrl_program
                    .execute(vrl_context)
                    .map_err(ProgramResolutionError::ExecutionFailed)?;

                T::from_vrl_value(result_value).map_err(ProgramResolutionError::ConversionFailed)
            }
        }
    }
}

/// Type alias for a Duration that can be either static or computed via expression
pub type DurationOrProgram = ValueOrProgram<Duration>;

/// Type alias for a Boolean that can be either static or computed via expression
pub type BooleanOrProgram = ValueOrProgram<bool>;

/// Type alias for a String that can be either static or computed via expression
///
/// Useful for endpoints, URLs, or any string configuration that can be dynamic
pub type StringOrProgram = ValueOrProgram<String>;
