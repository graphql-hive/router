use std::{collections::{BTreeMap}, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use hive_router_config::traffic_shaping::TrafficShapingExecutorConfig;
use http::Uri;
use http_body_util::Full;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use tokio::sync::{OnceCell, Semaphore};
use tracing::warn;

use crate::{execution::plan::ClientRequestDetails, executors::{
    common::{HttpExecutionRequest, HttpExecutionResponse, SubgraphExecutor},
    dedupe::{ABuildHasher, SharedResponse},
    error::SubgraphExecutorError, http::HTTPSubgraphExecutor,
}};
use vrl::compiler::Program as VrlProgram;

use vrl::{
    compiler::TargetValue as VrlTargetValue,
    core::Value as VrlValue,
    prelude::{state::RuntimeState as VrlState, Context as VrlContext, TimeZone as VrlTimeZone},
    value::Secrets as VrlSecrets,
};
use vrl::{
    compiler::compile as vrl_compile,
    stdlib::all as vrl_build_functions,
};

pub struct ExpressionHTTPExecutor {
    pub default_endpoint: Uri,
    pub expression: Box<VrlProgram>,
    pub http_client: Arc<Client<HttpsConnector<HttpConnector>, Full<Bytes>>>,
    pub traffic_shaping_config: Arc<TrafficShapingExecutorConfig>,
    pub in_flight_requests: Arc<DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>>,
    pub semaphores_by_origin: Arc<DashMap<String, Arc<Semaphore>>>,
    pub executor_map: Arc<DashMap<Uri, Arc<HTTPSubgraphExecutor>>>,
}

impl ExpressionHTTPExecutor {
    pub fn try_new(
        default_endpoint_str: &str,
        expression_str: &str,
        http_client: Arc<Client<HttpsConnector<HttpConnector>, Full<Bytes>>>,
        traffic_shaping_config: Arc<TrafficShapingExecutorConfig>,
        in_flight_requests: Arc<DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>>,
        semaphores_by_origin: Arc<DashMap<String, Arc<Semaphore>>>,
    ) -> Result<Self, SubgraphExecutorError> {
        let default_endpoint = default_endpoint_str.parse::<Uri>().map_err(|e| {
            SubgraphExecutorError::EndpointParseFailure(
                default_endpoint_str.to_string(),
                e.to_string(),
            )
        })?;
        let vrl_functions = vrl_build_functions();
        let compilation_result = vrl_compile(expression_str, &vrl_functions).map_err(|e| {
            SubgraphExecutorError::VrlCompileError(
                e
                    .errors()
                    .into_iter()
                    .map(|d| d.code.to_string() + ": " + &d.message)
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })?;
        Ok(Self {
            default_endpoint,
            expression: Box::new(compilation_result.program),
            http_client,
            traffic_shaping_config,
            in_flight_requests,
            semaphores_by_origin,
            executor_map: Arc::new(DashMap::new()),
        })
    }
}

struct ExpressionContext<'a> {
    client_request: &'a ClientRequestDetails<'a>,
}

impl From<&ExpressionContext<'_>> for VrlValue {
    fn from(ctx: &ExpressionContext) -> Self {
        // .request
        let request_value: Self = ctx.client_request.into();

        Self::Object(BTreeMap::from([
            ("request".into(), request_value),
        ]))
    }
}

fn warn_unsupported_conversion_option<T>(type_name: &str) -> Option<T> {
    warn!(
        "Cannot convert VRL {} value to a url value. Please convert it to a string first.",
        type_name
    );

    None
}

fn vrl_value_to_uri(value: VrlValue) -> Option<Uri> {
    match value {
        VrlValue::Bytes(bytes) => Uri::from_maybe_shared(bytes).ok(),
        VrlValue::Float(_) => warn_unsupported_conversion_option("Float"),
        VrlValue::Boolean(_) => warn_unsupported_conversion_option("Boolean"),
        VrlValue::Integer(_) => warn_unsupported_conversion_option("Integer"),
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

impl ExpressionHTTPExecutor {
    fn resolve_endpoint(&self, ctx: &ExpressionContext) -> Uri {
        let mut target = VrlTargetValue {
            value: ctx.into(),
            metadata: VrlValue::Object(BTreeMap::new()),
            secrets: VrlSecrets::default(),
        };

        let mut state = VrlState::default();
        let timezone = VrlTimeZone::default();
        let mut ctx = VrlContext::new(&mut target, &mut state, &timezone);
        let value = self.expression.resolve(&mut ctx);
        match value {
            Ok(v) => {
                if let Some(uri) = vrl_value_to_uri(v) {
                    uri
                } else {
                    warn!("Expression did not evaluate to a valid URI, falling back to default endpoint");
                    self.default_endpoint.clone()
                }
            }
            Err(err) => {
                warn!("Failed to evaluate expression: {}, falling back to default endpoint", err);
                self.default_endpoint.clone()
            }
        }
    }
}

#[async_trait]
impl SubgraphExecutor for ExpressionHTTPExecutor {
    async fn execute<'a>(
        &self,
        execution_request: HttpExecutionRequest<'a>,
    ) -> HttpExecutionResponse {
        let ctx = ExpressionContext {
            client_request: execution_request.client_request,
        };
        let endpoint = self.resolve_endpoint(&ctx);
        let executor = match self.executor_map.get(&endpoint) {
            Some(executor) => executor.clone(),
            None => {
                let new_executor = HTTPSubgraphExecutor::new(
                    endpoint.clone(),
                    self.http_client.clone(),
                    self.semaphores_by_origin.clone(),
                    self.traffic_shaping_config.clone(),
                    self.in_flight_requests.clone(),
                );
                let executor_arc = Arc::new(new_executor);
                self.executor_map
                    .insert(endpoint, executor_arc.clone());
                executor_arc
            }
        };
        executor.execute(execution_request).await
    }
}
