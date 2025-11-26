use once_cell::sync::Lazy;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::time::Duration;

use hive_router_config::traffic_shaping::DurationOrExpression;
use vrl::{
    compiler::{compile as vrl_compile, Program as VrlProgram, TargetValue as VrlTargetValue},
    core::Value as VrlValue,
    prelude::{
        state::RuntimeState as VrlState, Context as VrlContext, Function, TimeZone as VrlTimeZone,
    },
    value::Secrets as VrlSecrets,
};

use crate::expressions::error::{ExpressionCompileError, ExpressionExecutionError};

static VRL_FUNCTIONS: Lazy<Vec<Box<dyn Function>>> = Lazy::new(vrl::stdlib::all);
static VRL_TIMEZONE: Lazy<VrlTimeZone> = Lazy::new(VrlTimeZone::default);

/// This trait provides a unified way to convert VRL values to specific Rust types.
pub trait FromVrlValue: Sized {
    /// Associated error type for this conversion
    type Error: std::error::Error + Send + Sync + 'static;

    /// Convert a VRL value to this type
    /// - `value` - The VRL value to convert
    fn from_vrl_value(value: VrlValue) -> Result<Self, Self::Error>;
}

/// This trait provides a convenient method to compile expressions directly on string types.
pub trait CompileExpression {
    /// Compile a VRL expression string into an executable program
    /// - `functions` - Optional custom functions; if None, uses standard VRL functions
    fn compile_expression(
        &self,
        functions: Option<&[Box<dyn Function>]>,
    ) -> Result<VrlProgram, ExpressionCompileError>;
}

impl CompileExpression for str {
    fn compile_expression(
        &self,
        functions: Option<&[Box<dyn Function>]>,
    ) -> Result<VrlProgram, ExpressionCompileError> {
        let functions = functions.unwrap_or(&VRL_FUNCTIONS);

        let compilation_result = vrl_compile(self, functions).map_err(|diagnostics| {
            ExpressionCompileError::new(
                self.to_string(),
                // Format diagnostics into a human-readable string like this:
                // error[E203]: syntax error
                //   ┌─ :1:23
                //   │
                // 1 │ if (.request.headerss["x-timeout"] == "short") {
                //   │                       ^^^^^^^^^^^
                //   │                       │
                //   │                       unexpected syntax token: "StringLiteral"
                //   │                       expected one of: "integer literal"
                //   │
                //   = see language documentation at https://vrl.dev
                //   = try your code in the VRL REPL, learn more at https://vrl.dev/examples
                vrl::diagnostic::Formatter::new(self, diagnostics).to_string(),
            )
        })?;

        Ok(compilation_result.program)
    }
}

/// Provides a convenient `.execute()` method on VRL `Program` types
/// that handles all the boilerplate of setting up execution context,
/// target values, and error handling.
pub trait ExecutableProgram {
    fn execute(&self, value: VrlValue) -> Result<VrlValue, ExpressionExecutionError>;
}

impl ExecutableProgram for VrlProgram {
    #[inline]
    fn execute(&self, value: VrlValue) -> Result<VrlValue, ExpressionExecutionError> {
        let mut target = VrlTargetValue {
            value,
            metadata: VrlValue::Object(BTreeMap::new()),
            secrets: VrlSecrets::default(),
        };

        let mut state = VrlState::default();
        let mut ctx = VrlContext::new(&mut target, &mut state, &VRL_TIMEZONE);

        self.resolve(&mut ctx).map_err(ExpressionExecutionError)
    }
}

/// Errors that can occur during program resolution
#[derive(Debug, thiserror::Error)]
pub enum ProgramResolutionError<T: std::error::Error> {
    #[error("Failed to execute expression: {0}")]
    ExecutionFailed(#[source] ExpressionExecutionError),

    #[error("Failed to convert result: {0}")]
    ConversionFailed(#[source] T),
}

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
    /// - `vrl_context` - The VRL value context for expression execution
    #[inline]
    pub fn resolve(&self, vrl_context: VrlValue) -> Result<T, ProgramResolutionError<T::Error>> {
        match self {
            ValueOrProgram::Value(v) => Ok(v.clone()),
            ValueOrProgram::Program(vrl_program) => {
                let result_value = vrl_program
                    .execute(vrl_context)
                    .map_err(ProgramResolutionError::ExecutionFailed)?;

                T::from_vrl_value(result_value)
                    .map_err(ProgramResolutionError::ConversionFailed)
            }
        }
    }
}

impl ValueOrProgram<Duration> {
    pub fn compile(
        config: &DurationOrExpression,
        fns: Option<&[Box<dyn Function>]>,
    ) -> Result<Self, ExpressionCompileError> {
        match config {
            DurationOrExpression::Duration(dur) => Ok(ValueOrProgram::Value(*dur)),
            DurationOrExpression::Expression { expression } => {
                let program = expression.as_str().compile_expression(fns)?;
                Ok(ValueOrProgram::Program(Box::new(program)))
            }
        }
    }
}
