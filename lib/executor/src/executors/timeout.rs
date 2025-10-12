use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use hive_router_config::traffic_shaping::SubgraphTimeoutConfig;
use tracing::warn;
use vrl::compiler::Program as VrlProgram;

use crate::executors::common::{
    HttpExecutionRequest, HttpExecutionResponse, SubgraphExecutor, SubgraphExecutorBoxedArc,
};
use crate::executors::error::error_to_graphql_bytes;
use crate::{execution::plan::ClientRequestDetails, executors::error::SubgraphExecutorError};
use vrl::{
    compiler::TargetValue as VrlTargetValue,
    core::Value as VrlValue,
    prelude::{state::RuntimeState as VrlState, Context as VrlContext, TimeZone as VrlTimeZone},
    value::Secrets as VrlSecrets,
};

use vrl::{compiler::compile as vrl_compile, stdlib::all as vrl_build_functions};

#[derive(Debug)]
pub enum TimeoutSource {
    Expression(Box<VrlProgram>),
    Duration(Duration),
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
        VrlValue::Integer(i) => {
            if i < 0 {
                warn!("Cannot convert negative integer ({}) to Duration.", i);
                None
            } else {
                Some(Duration::from_millis(i as u64))
            }
        }
        VrlValue::Bytes(_) => warn_unsupported_conversion_option("Bytes"),
        VrlValue::Float(_) => warn_unsupported_conversion_option("Float"),
        VrlValue::Boolean(_) => warn_unsupported_conversion_option("Boolean"),
        VrlValue::Array(_) => warn_unsupported_conversion_option("Array"),
        VrlValue::Regex(_) => warn_unsupported_conversion_option("Regex"),
        VrlValue::Timestamp(_) => warn_unsupported_conversion_option("Timestamp"),
        VrlValue::Object(_) => warn_unsupported_conversion_option("Object"),
        VrlValue::Null => {
            warn!("Cannot convert VRL Null value to a Duration value.");
            None
        }
    }
}

pub struct TimeoutExecutor {
    pub endpoint: http::Uri,
    pub timeout: TimeoutSource,
    pub executor: SubgraphExecutorBoxedArc,
}

impl TimeoutExecutor {
    pub fn try_new(
        endpoint: http::Uri,
        timeout_config: &SubgraphTimeoutConfig,
        executor: SubgraphExecutorBoxedArc,
    ) -> Result<Self, SubgraphExecutorError> {
        let timeout = match timeout_config {
            SubgraphTimeoutConfig::Duration(dur) => TimeoutSource::Duration(*dur),
            SubgraphTimeoutConfig::Expression(expr) => {
                // Compile the VRL expression into a Program
                let functions = vrl_build_functions();
                let compilation_result = vrl_compile(expr, &functions).map_err(|diagnostics| {
                    SubgraphExecutorError::TimeoutExpressionParseFailure(
                        diagnostics
                            .errors()
                            .into_iter()
                            .map(|d| d.code.to_string() + ": " + &d.message)
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                })?;
                TimeoutSource::Expression(Box::new(compilation_result.program))
            }
        };
        Ok(Self {
            endpoint,
            timeout,
            executor,
        })
    }
    pub fn get_timeout_duration<'a>(
        &self,
        client_request: &'a ClientRequestDetails<'a>,
    ) -> Option<Duration> {
        let expression_context = ExpressionContext { client_request };

        match &self.timeout {
            TimeoutSource::Duration(dur) => Some(*dur),
            TimeoutSource::Expression(program) => {
                let mut target = VrlTargetValue {
                    value: VrlValue::from(&expression_context),
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
        }
    }
}

#[async_trait]
impl SubgraphExecutor for TimeoutExecutor {
    async fn execute<'a>(
        &self,
        execution_request: HttpExecutionRequest<'a>,
    ) -> HttpExecutionResponse {
        let timeout = self.get_timeout_duration(execution_request.client_request);
        let execution = self.executor.execute(execution_request);
        if let Some(timeout) = timeout {
            match tokio::time::timeout(timeout, execution).await {
                Ok(response) => response,
                Err(_) => HttpExecutionResponse {
                    body: error_to_graphql_bytes(
                        &self.endpoint,
                        SubgraphExecutorError::RequestTimeout(timeout),
                    ),
                    headers: Default::default(),
                },
            }
        } else {
            execution.await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::{extract::State, http::Response, Router};
    use hive_router_config::parse_yaml_config;
    use http::Method;
    use ntex_http::HeaderMap;

    use crate::{
        execution::plan::{ClientRequestDetails, OperationDetails},
        executors::{
            common::{HttpExecutionRequest, HttpExecutionResponse, SubgraphExecutor},
            map::from_traffic_shaping_config_to_client,
            timeout::TimeoutExecutor,
        },
    };

    struct MockExecutor {}

    #[async_trait]
    impl SubgraphExecutor for MockExecutor {
        async fn execute<'a>(
            &self,
            _execution_request: HttpExecutionRequest<'a>,
        ) -> HttpExecutionResponse {
            HttpExecutionResponse {
                body: Default::default(),
                headers: Default::default(),
            }
        }
    }

    #[test]
    fn get_timeout_duration_from_expression() {
        use std::time::Duration;

        use hive_router_config::traffic_shaping::SubgraphTimeoutConfig;

        let timeout_config = SubgraphTimeoutConfig::Expression(
            r#"
            if .request.operation.type == "mutation" {
                10000
            } else {
                5000
            }
            "#
            .to_string(),
        );

        let mock_executor = MockExecutor {}.to_boxed_arc();

        let timeout_executor = TimeoutExecutor::try_new(
            "http://example.com/graphql".parse().unwrap(),
            &timeout_config,
            mock_executor,
        )
        .unwrap();

        let headers = HeaderMap::new();

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
        let duration_query = timeout_executor.get_timeout_duration(&client_request_query);
        assert_eq!(
            duration_query,
            Some(Duration::from_millis(5000)),
            "Expected 5000ms for query"
        );

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

        let duration_mutation = timeout_executor.get_timeout_duration(&client_request_mutation);
        assert_eq!(
            duration_mutation,
            Some(Duration::from_millis(10000)),
            "Expected 10000ms for mutation"
        );
    }

    #[test]
    fn get_timeout_duration_from_fixed_duration() {
        let yaml_str = r#"
           traffic_shaping:
             all:
                timeout: 
                    duration: 7s
        "#;
        let config = parse_yaml_config(yaml_str.to_string()).unwrap();
        let mock_executor = MockExecutor {}.to_boxed_arc();
        let timeout_executor = TimeoutExecutor::try_new(
            "http://example.com/graphql".parse().unwrap(),
            &config.traffic_shaping.all.timeout.unwrap(),
            mock_executor,
        )
        .unwrap();

        let headers = HeaderMap::new();
        let client_request = ClientRequestDetails {
            operation: OperationDetails {
                name: Some("TestQuery".to_string()),
                kind: "query",
                query: "query TestQuery { field }".into(),
            },
            url: "http://example.com/graphql".parse().unwrap(),
            headers: &headers,
            method: Method::POST,
        };
        let duration = timeout_executor.get_timeout_duration(&client_request);
        assert_eq!(duration, Some(std::time::Duration::from_millis(7000)));
    }

    #[tokio::test]
    async fn cancels_http_request_when_timeout_expires() {
        /**
         * We will test here that when the timeout expires, the request is cancelled on the server-end as well.
         * For that, we will create a server that sets a flag when the request is dropped/cancelled.
         */
        use std::sync::Arc;

        use http::Method;

        let (tx, mut rx) = tokio::sync::broadcast::channel(16);

        struct AppState {
            tx: Arc<tokio::sync::broadcast::Sender<Duration>>,
        }

        let app_state = AppState { tx: Arc::new(tx) };

        let app_state_arc = Arc::new(app_state);

        struct CancelOnDrop {
            start: std::time::Instant,
            tx: Arc<tokio::sync::broadcast::Sender<Duration>>,
        }

        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                self.tx.send(self.start.elapsed()).unwrap();
            }
        }

        #[axum::debug_handler]
        async fn handler(State(state): State<Arc<AppState>>) -> Response<String> {
            let _cancel_on_drop = CancelOnDrop {
                start: std::time::Instant::now(),
                tx: state.tx.clone(),
            };
            // Never resolve the request, just wait until it's cancelled
            let fut = futures::future::pending::<Response<String>>();
            fut.await
        }

        println!("Starting server...");
        let app = Router::new()
            .fallback(handler)
            .with_state(app_state_arc.clone());
        println!("Router created, binding to port...");
        let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        println!("Listener bound, starting server...");
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("Server error: {}", e);
            }
        });
        println!("Server started on {}", addr);
        let graphql_path = "graphql";
        let endpoint: http::Uri = format!("http://{}/{}", addr, graphql_path).parse().unwrap();
        println!("Endpoint: {}", endpoint);

        let config = r#"
           traffic_shaping:
             all:
                timeout: 
                    duration: 5s
        "#;

        let config = hive_router_config::parse_yaml_config(config.to_string()).unwrap();
        let http_client = from_traffic_shaping_config_to_client(&config.traffic_shaping.all);
        let http_executor = crate::executors::http::HTTPSubgraphExecutor::new(
            endpoint.clone(),
            http_client,
            Arc::new(tokio::sync::Semaphore::new(10)),
            Arc::new(config.traffic_shaping.all.clone()),
            Default::default(),
        );
        let timeout_executor = TimeoutExecutor::try_new(
            endpoint,
            &config.traffic_shaping.all.timeout.unwrap(),
            http_executor.to_boxed_arc(),
        )
        .unwrap();

        let headers = HeaderMap::new();
        let client_request = ClientRequestDetails {
            operation: OperationDetails {
                name: Some("TestQuery".to_string()),
                kind: "query",
                query: "query TestQuery { field }".into(),
            },
            url: "http://example.com/graphql".parse().unwrap(),
            headers: &headers,
            method: Method::POST,
        };

        let execution_request = HttpExecutionRequest {
            operation_name: Some("TestQuery"),
            query: r#"{ field }"#,
            variables: None,
            representations: None,
            headers: http::HeaderMap::new(),
            client_request: &client_request,
            dedupe: true,
        };

        println!("Sending request to executor with 5s timeout...");
        let response = timeout_executor.execute(execution_request).await;

        println!("Received response from executor.");
        assert!(
            response
                .body
                .starts_with(b"{\"errors\":[{\"message\":\"Failed to execute request to subgraph"),
            "Expected error response due to timeout"
        );

        println!("Waiting to see if server was notified of cancellation...");

        // Wait for the server to be notified that the request was cancelled
        let elapsed = rx.recv().await.unwrap();
        println!("Server was notified of cancellation after {:?}", elapsed);
        assert!(
            elapsed >= Duration::from_secs_f32(4.9),
            "Expected server to be notified of cancellation after at least 5s, but was {:?}",
            elapsed
        );

        println!("Test completed.");
    }
}
