pub use hive_console_sdk::expressions::error::{ExpressionCompileError, ProgramResolutionError};
pub use hive_console_sdk::expressions::lib::{
    CompileExpression, ExecutableProgram, FromVrlValue, ProgramHints, ToVrlValue,
};
use std::time::Duration;
pub use vrl::{compiler::Program as VrlProgram, core::Value as VrlValue};

pub use vrl::prelude::{ExpressionError as VrlExpressionError, Function as VrlFunction};

/// Generic enum for a value that can be either static or computed via VRL expression
#[derive(Clone)]
pub enum ValueOrProgram<T> {
    /// A statically-known value
    Value(T),
    /// A VRL program that computes the value at runtime, along with hints about which
    /// fields are accessed during execution (used to optimize context construction).
    Program(Box<VrlProgram>, ProgramHints),
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
            ValueOrProgram::Program(vrl_program, _) => {
                let vrl_context = vrl_context_fn();
                let result_value = vrl_program
                    .execute(vrl_context)
                    .map_err(ProgramResolutionError::ExecutionFailed)?;

                T::from_vrl_value(result_value).map_err(ProgramResolutionError::ConversionFailed)
            }
        }
    }

    /// Resolve using hints to allow the context function to selectively build the context.
    #[inline]
    pub fn resolve_with_hints<F>(
        &self,
        vrl_context_fn: F,
    ) -> Result<T, ProgramResolutionError<T::Error>>
    where
        F: FnOnce(&ProgramHints) -> VrlValue,
    {
        match self {
            ValueOrProgram::Value(v) => Ok(v.clone()),
            ValueOrProgram::Program(vrl_program, hints) => {
                let vrl_context = vrl_context_fn(hints);
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
