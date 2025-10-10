use std::collections::BTreeMap;
use std::time::Duration;

use bytes::Bytes;
use futures::TryFutureExt;
use hive_router_config::traffic_shaping::HTTPTimeoutConfig;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Incoming;
use tracing::warn;
use vrl::compiler::Program as VrlProgram;
use vrl::diagnostic::DiagnosticList;

use crate::executors::http::HTTPSubgraphExecutor;
use crate::{execution::plan::ClientRequestDetails, executors::error::SubgraphExecutorError};
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

fn get_timeout_duration<'a>(
    timeout: &Option<HTTPTimeout>,
    expression_context: &ExpressionContext<'a>,
) -> Option<Duration> {
    timeout.as_ref().and_then(|timeout| match timeout {
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
                    warn!(
                        "Failed to evaluate timeout expression: {:#?}, falling back to no timeout.",
                        err
                    );
                    None
                }
            }
        }
    })
}

impl HTTPSubgraphExecutor {
    pub fn get_timeout_duration<'a>(
        &self,
        client_request: &'a ClientRequestDetails<'a>,
    ) -> Option<Duration> {
        let expression_context = ExpressionContext { client_request };
        get_timeout_duration(&self.timeout, &expression_context)
    }

    pub async fn send_request_with_timeout(
        &self,
        req: Request<Full<Bytes>>,
        timeout: Duration,
    ) -> Result<Response<Incoming>, SubgraphExecutorError> {
        let request_op = self.send_request_to_client(req);

        tokio::time::timeout(timeout, request_op)
            .map_err(|_| SubgraphExecutorError::RequestTimeout(self.endpoint.to_string(), timeout))
            .await?
    }
}

#[cfg(test)]
mod tests {
    use http::Method;
    use ntex_http::HeaderMap;

    use crate::{
        execution::plan::{ClientRequestDetails, OperationDetails},
        executors::timeout::get_timeout_duration,
    };

    #[test]
    fn get_timeout_duration_from_expression() {
        use std::time::Duration;

        use hive_router_config::traffic_shaping::HTTPTimeoutConfig;

        use crate::executors::timeout::HTTPTimeout;

        let timeout_config = HTTPTimeoutConfig::Expression(
            r#"
            if .request.operation.type == "mutation" {
                10000
            } else {
                5000
            }
            "#
            .to_string(),
        );

        let timeout = HTTPTimeout::try_from(&timeout_config).expect("Failed to create timeout");
        let headers = HeaderMap::new();

        let client_request_mutation = crate::execution::plan::ClientRequestDetails {
            operation: OperationDetails {
                name: Some("TestMutation".to_string()),
                kind: "mutation",
                query: "mutation TestMutation { doSomething }".into(),
            },
            url: "http://example.com/graphql".parse().unwrap(),
            headers: &headers,
            method: Method::POST,
        };

        let timeout = Some(timeout);

        let client_request_query = ClientRequestDetails {
            operation: OperationDetails {
                name: Some("TestQuery".to_string()),
                kind: "query",
                query: "query TestQuery { field }".into(),
            },
            url: "http://example.com/graphql".parse().unwrap(),
            headers: &headers,
            method: Method::POST,
        };

        let query_ctx = crate::executors::timeout::ExpressionContext {
            client_request: &client_request_query,
        };
        let duration_query = get_timeout_duration(&timeout, &query_ctx);
        assert_eq!(
            duration_query,
            Some(Duration::from_millis(5000)),
            "Expected 5000ms for query"
        );

        let mutation_ctx = crate::executors::timeout::ExpressionContext {
            client_request: &client_request_mutation,
        };
        let duration_mutation = get_timeout_duration(&timeout, &mutation_ctx);
        assert_eq!(
            duration_mutation,
            Some(Duration::from_millis(10000)),
            "Expected 10000ms for mutation"
        );
    }
}
