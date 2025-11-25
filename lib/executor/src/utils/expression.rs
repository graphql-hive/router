use once_cell::sync::Lazy;
use std::collections::BTreeMap;
use vrl::{
    compiler::{compile as vrl_compile, Program as VrlProgram, TargetValue as VrlTargetValue},
    core::Value as VrlValue,
    prelude::{
        state::RuntimeState as VrlState, Context as VrlContext, ExpressionError, Function,
        TimeZone as VrlTimeZone,
    },
    stdlib::all as vrl_build_functions,
    value::Secrets as VrlSecrets,
};

static VRL_FUNCTIONS: Lazy<Vec<Box<dyn Function>>> = Lazy::new(vrl_build_functions);
static VRL_TIMEZONE: Lazy<VrlTimeZone> = Lazy::new(VrlTimeZone::default);

pub fn compile_expression(
    expression: &str,
    functions: Option<&[Box<dyn Function>]>,
) -> Result<VrlProgram, String> {
    let functions = functions.unwrap_or(&VRL_FUNCTIONS);

    let compilation_result = vrl_compile(expression, functions).map_err(|diagnostics| {
        diagnostics
            .errors()
            .iter()
            .map(|d| {
                format!(
                    "https://vector.dev/docs/reference/vrl/errors/#{} - {}",
                    d.code, d.message
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    })?;

    Ok(compilation_result.program)
}

pub fn execute_expression_with_value(
    program: &VrlProgram,
    value: VrlValue,
) -> Result<VrlValue, ExpressionError> {
    let mut target = VrlTargetValue {
        value,
        metadata: VrlValue::Object(BTreeMap::new()),
        secrets: VrlSecrets::default(),
    };

    let mut state = VrlState::default();
    let mut ctx = VrlContext::new(&mut target, &mut state, &VRL_TIMEZONE);

    program.resolve(&mut ctx)
}
