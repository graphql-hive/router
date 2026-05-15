//! Runtime form of [`crate::agent::config::UsageReportingExclude`].
//!
//! [`Exclude::from_config`] is what turns the deserialized config into a
//! ready-to-use filter, compiling any VRL expression up front so config
//! errors surface synchronously when the agent is built. The agent then
//! calls [`Exclude::should_exclude`] for every report before sampling.

use std::collections::HashSet;

use vrl::compiler::Program as VrlProgram;

use crate::agent::config::UsageReportingExclude;
use crate::agent::usage_agent::{
    get_vrl_value_from_execution_report_and_request, ExecutionReport, RequestDetails,
};
use crate::expressions::lib::FromVrlValue;
use crate::expressions::values::boolean::BooleanConversionError;
use crate::expressions::CompileExpression;
use crate::expressions::ExecutableProgram;
use crate::expressions::{ExpressionCompileError, ExpressionExecutionError};

#[derive(Debug, thiserror::Error)]
pub enum ExcludeError {
    #[error("failed to compile exclude expression: {0}")]
    Compile(#[from] ExpressionCompileError),
    #[error("failed to execute exclude expression: {0}")]
    Execute(#[from] ExpressionExecutionError),
    #[error("failed to convert exclude expression result to boolean: {0}")]
    ResultConversion(#[from] BooleanConversionError),
}

/// Runtime exclusion filter: either a compiled VRL expression or a static
/// set of operation names. The compiled `VrlProgram` is boxed because it
/// is several hundred bytes and would otherwise dominate the size of every
/// [`Exclude`] value.
pub enum Exclude {
    Expression { program: Box<VrlProgram> },
    OperationNames(HashSet<String>),
}

impl Exclude {
    /// Build a runtime [`Exclude`] from a deserialized config value. VRL
    /// programs are compiled here so configuration errors are reported
    /// when the agent is built, not on the first incoming report.
    pub fn from_config(config: &UsageReportingExclude) -> Result<Self, ExcludeError> {
        match config {
            UsageReportingExclude::Expression { expression } => {
                let program = expression.compile_expression(None)?;
                Ok(Self::Expression {
                    program: Box::new(program),
                })
            }
            UsageReportingExclude::OperationNames(names) => {
                Ok(Self::OperationNames(names.iter().cloned().collect()))
            }
        }
    }

    /// Decide whether a report should be excluded.
    ///
    /// Returns `Ok(true)` to drop the report, `Ok(false)` to keep it.
    pub fn should_exclude(
        &self,
        report: &ExecutionReport,
        request: Option<&RequestDetails>,
    ) -> Result<bool, ExcludeError> {
        match self {
            Self::Expression { program } => {
                let value =
                    get_vrl_value_from_execution_report_and_request(report, request.cloned());
                let result = program.execute(value)?;
                Ok(bool::from_vrl_value(result)?)
            }
            Self::OperationNames(names) => Ok(report
                .operation_name
                .as_deref()
                .map(|name| names.contains(name))
                .unwrap_or(false)),
        }
    }
}
