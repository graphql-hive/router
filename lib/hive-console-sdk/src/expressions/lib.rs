use std::collections::BTreeMap;
use std::sync::LazyLock;

use vrl::{
    compiler::{compile as vrl_compile, Program as VrlProgram, TargetValue as VrlTargetValue},
    core::Value as VrlValue,
    prelude::{
        state::RuntimeState as VrlState, Context as VrlContext, Function, TimeZone as VrlTimeZone,
    },
    value::Secrets as VrlSecrets,
};

use crate::expressions::{
    error::{ExpressionCompileError, ExpressionExecutionError},
    functions::env::Env,
};

static VRL_FUNCTIONS: LazyLock<Vec<Box<dyn Function>>> = LazyLock::new(|| {
    let mut funcs = vrl::stdlib::all();
    // Our custom functions:
    funcs.push(Box::new(Env));
    funcs
});
static VRL_TIMEZONE: LazyLock<VrlTimeZone> = LazyLock::new(VrlTimeZone::default);

/// This trait provides a unified way to convert VRL values to specific Rust types.
pub trait FromVrlValue: Sized {
    /// Associated error type for this conversion
    type Error: std::error::Error + Send + Sync + 'static;

    /// Convert a VRL value to this type
    /// - `value` - The VRL value to convert
    fn from_vrl_value(value: VrlValue) -> Result<Self, Self::Error>;
}

/// This trait provides a convenient method to convert sonic_rs Values to VRL Values.
pub trait ToVrlValue {
    /// Convert a sonic_rs Value to a VRL Value
    fn to_vrl_value(&self) -> VrlValue;
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

        Ok(self.resolve(&mut ctx)?)
    }
}
