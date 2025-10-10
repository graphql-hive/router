use std::collections::BTreeMap;
use std::time::Duration;

use hive_router_config::traffic_shaping::HTTPTimeoutConfig;
use tracing::warn;
use vrl::compiler::Program as VrlProgram;
use vrl::diagnostic::DiagnosticList;

use crate::execution::plan::ClientRequestDetails;
use crate::executors::http::HTTPSubgraphExecutor;
use vrl::{
    compiler::TargetValue as VrlTargetValue,
    core::Value as VrlValue,
    prelude::{state::RuntimeState as VrlState, Context as VrlContext, TimeZone as VrlTimeZone},
    value::Secrets as VrlSecrets,
};

use vrl::{compiler::compile as vrl_compile, stdlib::all as vrl_build_functions};

#[derive(Debug)]
pub enum HTTPTimeout {
    Expression(Box<VrlProgram>),
    Duration(Duration),
}

impl TryFrom<&HTTPTimeoutConfig> for HTTPTimeout {
    type Error = DiagnosticList;
    fn try_from(timeout_config: &HTTPTimeoutConfig) -> Result<HTTPTimeout, DiagnosticList> {
        match timeout_config {
            HTTPTimeoutConfig::Duration(dur) => Ok(HTTPTimeout::Duration(*dur)),
            HTTPTimeoutConfig::Expression(expr) => {
                // Compile the VRL expression into a Program
                let functions = vrl_build_functions();
                let compilation_result = vrl_compile(expr, &functions)?;
                Ok(HTTPTimeout::Expression(Box::new(
                    compilation_result.program,
                )))
            }
        }
    }
}

pub struct ExpressionContext<'a> {
    pub client_request: &'a ClientRequestDetails<'a>,
}

impl From<&ExpressionContext<'_>> for VrlValue {
    fn from(ctx: &ExpressionContext) -> Self {
        // .request
        let request_value: Self = ctx.client_request.into();

        Self::Object(BTreeMap::from([("request".into(), request_value)]))
    }
}

fn warn_unsupported_conversion_option<T>(type_name: &str) -> Option<T> {
    warn!(
        "Cannot convert VRL {} value to a Duration value. Please convert it to a number first.",
        type_name
    );

    None
}

fn vrl_value_to_duration(value: VrlValue) -> Option<Duration> {
    match value {
        VrlValue::Integer(i) => Some(Duration::from_millis(u64::from_ne_bytes(i.to_ne_bytes()))),
        VrlValue::Bytes(_) => warn_unsupported_conversion_option("Bytes"),
        VrlValue::Float(_) => warn_unsupported_conversion_option("Float"),
        VrlValue::Boolean(_) => warn_unsupported_conversion_option("Boolean"),
        VrlValue::Array(_) => warn_unsupported_conversion_option("Array"),
        VrlValue::Regex(_) => warn_unsupported_conversion_option("Regex"),
        VrlValue::Timestamp(_) => warn_unsupported_conversion_option("Timestamp"),
        VrlValue::Object(_) => warn_unsupported_conversion_option("Object"),
        VrlValue::Null => {
            warn!("Cannot convert VRL Null value to a url value.");
            None
        }
    }
}

impl HTTPSubgraphExecutor {
    pub fn get_timeout_duration<'a>(
        &self,
        expression_context: &ExpressionContext<'a>,
    ) -> Option<Duration> {
        self.timeout.as_ref().and_then(|timeout| {
            match timeout {
                HTTPTimeout::Duration(dur) => Some(*dur),
                HTTPTimeout::Expression(program) => {
                    let mut target = VrlTargetValue {
                        value: VrlValue::from(expression_context),
                        metadata: VrlValue::Object(BTreeMap::new()),
                        secrets: VrlSecrets::default(),
                    };

                    let mut state = VrlState::default();
                    let timezone = VrlTimeZone::default();
                    let mut ctx = VrlContext::new(&mut target, &mut state, &timezone);
                    match program.resolve(&mut ctx) {
                        Ok(resolved) => vrl_value_to_duration(resolved),
                        Err(err) => {
                            warn!("Failed to evaluate timeout expression: {:#?}, falling back to no timeout.", err);
                            None
                        }
                    }
                },
            }
        })
    }
}
