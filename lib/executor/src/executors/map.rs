use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};

use dashmap::DashMap;
use futures::{stream, stream::BoxStream};
use hive_router_config::{
    override_subgraph_urls::UrlOrExpression, subscriptions::SubscriptionProtocol,
    traffic_shaping::DurationOrExpression, HiveRouterConfig,
};
use hive_router_internal::expressions::vrl::compiler::Program as VrlProgram;
use hive_router_internal::expressions::vrl::core::Value as VrlValue;
use hive_router_internal::expressions::{CompileExpression, DurationOrProgram, ExecutableProgram};
use http::Uri;
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::{TokioExecutor, TokioTimer},
};
use tokio::sync::{OnceCell, Semaphore};
use tracing::error;

use crate::{
    execution::client_request_details::ClientRequestDetails,
    executors::{
        common::{SubgraphExecutionRequest, SubgraphExecutor, SubgraphExecutorBoxedArc},
        dedupe::ABuildHasher,
        error::SubgraphExecutorError,
        http::{HTTPSubgraphExecutor, HttpClient, HttpResponse},
        ws::WsSubgraphExecutor,
    },
    response::subgraph_response::SubgraphResponse,
};

type SubgraphName = String;
type SubgraphEndpoint = String;
type ExecutorsBySubgraphMap =
    DashMap<SubgraphName, DashMap<SubgraphEndpoint, SubgraphExecutorBoxedArc>>;
type StaticEndpointsBySubgraphMap = DashMap<SubgraphName, SubgraphEndpoint>;
type ExpressionEndpointsBySubgraphMap = HashMap<SubgraphName, VrlProgram>;
type TimeoutsBySubgraph = DashMap<SubgraphName, DurationOrProgram>;

struct ResolvedSubgraphConfig<'a> {
    client: Arc<HttpClient>,
    timeout_config: &'a DurationOrExpression,
    dedupe_enabled: bool,
}

pub type InflightRequestsMap = Arc<DashMap<u64, Arc<OnceCell<HttpResponse>>, ABuildHasher>>;

#[derive(Clone)]
pub struct SubgraphExecutorMap {
    http_executors_by_subgraph: ExecutorsBySubgraphMap,
    subscription_executors_by_subgraph: ExecutorsBySubgraphMap,
    /// Mapping from subgraph name to static endpoint for quick lookup
    /// based on subgraph SDL and static overrides from router's config.
    static_endpoints_by_subgraph: StaticEndpointsBySubgraphMap,
    /// Mapping from subgraph name to VRL expression program
    /// Only contains subgraphs with expression-based endpoint overrides
    expression_endpoints_by_subgraph: ExpressionEndpointsBySubgraphMap,
    timeouts_by_subgraph: TimeoutsBySubgraph,
    global_timeout: DurationOrProgram,
    config: Arc<HiveRouterConfig>,
    client: Arc<HttpClient>,
    semaphores_by_origin: DashMap<String, Arc<Semaphore>>,
    max_connections_per_host: usize,
    in_flight_requests: InflightRequestsMap,
}

fn build_https_executor() -> Result<HttpsConnector<HttpConnector>, SubgraphExecutorError> {
    HttpsConnectorBuilder::new()
        .with_native_roots()
        .map_err(|e| SubgraphExecutorError::NativeTlsCertificatesError(e.to_string()))
        .map(|b| b.https_or_http().enable_http1().enable_http2().build())
}

impl SubgraphExecutorMap {
    pub fn new(
        config: Arc<HiveRouterConfig>,
        global_timeout: DurationOrProgram,
    ) -> Result<Self, SubgraphExecutorError> {
        let client: HttpClient = Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(config.traffic_shaping.all.pool_idle_timeout)
            .pool_max_idle_per_host(config.traffic_shaping.max_connections_per_host)
            .build(build_https_executor()?);

        let max_connections_per_host = config.traffic_shaping.max_connections_per_host;

        Ok(SubgraphExecutorMap {
            http_executors_by_subgraph: Default::default(),
            subscription_executors_by_subgraph: Default::default(),
            static_endpoints_by_subgraph: Default::default(),
            expression_endpoints_by_subgraph: Default::default(),
            config,
            client: Arc::new(client),
            semaphores_by_origin: Default::default(),
            max_connections_per_host,
            in_flight_requests: Arc::new(DashMap::with_hasher(ABuildHasher::default())),
            timeouts_by_subgraph: Default::default(),
            global_timeout,
        })
    }

    pub fn from_http_endpoint_map(
        subgraph_endpoint_map: &HashMap<SubgraphName, String>,
        config: Arc<HiveRouterConfig>,
    ) -> Result<Self, SubgraphExecutorError> {
        let global_timeout = DurationOrProgram::compile(
            &config.traffic_shaping.all.request_timeout,
            None,
        )
        .map_err(|err| {
            SubgraphExecutorError::RequestTimeoutExpressionBuild("all".to_string(), err.diagnostics)
        })?;
        let mut subgraph_executor_map = SubgraphExecutorMap::new(config.clone(), global_timeout)?;

        for (subgraph_name, original_endpoint_str) in subgraph_endpoint_map.iter() {
            let endpoint_config = config
                .override_subgraph_urls
                .get_subgraph_url(subgraph_name);

            let endpoint_str = match endpoint_config {
                Some(UrlOrExpression::Url(url)) => url.clone(),
                Some(UrlOrExpression::Expression { expression }) => {
                    subgraph_executor_map
                        .register_endpoint_expression(subgraph_name, expression)?;
                    original_endpoint_str.clone()
                }
                None => original_endpoint_str.clone(),
            };

            subgraph_executor_map.register_static_endpoint(subgraph_name, &endpoint_str);
            subgraph_executor_map.register_executor(subgraph_name, &endpoint_str, None)?;
            subgraph_executor_map.register_subgraph_timeout(subgraph_name)?;
        }

        Ok(subgraph_executor_map)
    }

    pub async fn execute<'exec>(
        &self,
        subgraph_name: &'exec str,
        execution_request: SubgraphExecutionRequest<'exec>,
        client_request: &ClientRequestDetails<'exec>,
    ) -> SubgraphResponse<'exec> {
        match self.get_or_create_http_executor(subgraph_name, client_request) {
            Ok(executor) => {
                let timeout = self.resolve_subgraph_timeout(subgraph_name, client_request);

                match timeout {
                    Ok(timeout) => executor.execute(execution_request, timeout).await,
                    Err(err) => {
                        error!(
                            "Failed to resolve timeout for subgraph '{}': {}",
                            subgraph_name, err,
                        );
                        err.to_subgraph_response(subgraph_name)
                    }
                }
            }
            Err(err) => {
                error!(
                    "Subgraph executor error for subgraph '{}': {}",
                    subgraph_name, err,
                );
                err.to_subgraph_response(subgraph_name)
            }
        }
    }

    pub async fn subscribe<'exec>(
        &self,
        subgraph_name: &'exec str,
        execution_request: SubgraphExecutionRequest<'exec>,
        client_request: &ClientRequestDetails<'exec>,
    ) -> BoxStream<'static, SubgraphResponse<'static>> {
        match self.get_or_create_subscription_executor(subgraph_name, client_request) {
            Ok(executor) => {
                let timeout = self.resolve_subgraph_timeout(subgraph_name, client_request);

                match timeout {
                    Ok(timeout) => executor.subscribe(execution_request, timeout).await,
                    Err(err) => {
                        error!(
                            "Failed to resolve timeout for subgraph '{}': {}",
                            subgraph_name, err,
                        );
                        let response = err.to_subgraph_response(subgraph_name);
                        Box::pin(stream::once(async move { response }))
                    }
                }
            }
            Err(err) => {
                error!(
                    "Subgraph subscription executor error for subgraph '{}': {}",
                    subgraph_name, err,
                );
                let response = err.to_subgraph_response(subgraph_name);
                Box::pin(stream::once(async move { response }))
            }
        }
    }

    fn resolve_subgraph_timeout(
        &self,
        subgraph_name: &str,
        client_request: &ClientRequestDetails<'_>,
    ) -> Result<Option<Duration>, SubgraphExecutorError> {
        self.timeouts_by_subgraph
            .get(subgraph_name)
            .map(|t| {
                let global_timeout_duration =
                    resolve_timeout(&self.global_timeout, client_request, None, "all")?;
                resolve_timeout(
                    t.value(),
                    client_request,
                    Some(global_timeout_duration),
                    subgraph_name,
                )
            })
            .transpose()
    }

    fn resolve_endpoint(
        &self,
        subgraph_name: &str,
        client_request: &ClientRequestDetails<'_>,
    ) -> Result<String, SubgraphExecutorError> {
        if let Some(expression) = self.expression_endpoints_by_subgraph.get(subgraph_name) {
            let original_url_value = VrlValue::Bytes(
                self.static_endpoints_by_subgraph
                    .get(subgraph_name)
                    .map(|endpoint| endpoint.value().clone())
                    .ok_or_else(|| {
                        SubgraphExecutorError::StaticEndpointNotFound(subgraph_name.to_string())
                    })?
                    .into(),
            );

            let value = VrlValue::Object(BTreeMap::from([
                ("request".into(), client_request.into()),
                ("default".into(), original_url_value),
            ]));

            let endpoint_result = expression.execute(value).map_err(|err| {
                SubgraphExecutorError::EndpointExpressionResolutionFailure(
                    subgraph_name.to_string(),
                    err.to_string(),
                )
            })?;

            match endpoint_result.as_str() {
                Some(s) => Ok(s.to_string()),
                None => Err(SubgraphExecutorError::EndpointExpressionWrongType(
                    subgraph_name.to_string(),
                )),
            }
        } else {
            self.static_endpoints_by_subgraph
                .get(subgraph_name)
                .map(|e| e.value().clone())
                .ok_or_else(|| {
                    SubgraphExecutorError::StaticEndpointNotFound(subgraph_name.to_string())
                })
        }
    }

    fn get_semaphore_for_uri(&self, uri: &Uri) -> Arc<Semaphore> {
        let origin = format!(
            "{}://{}:{}",
            uri.scheme_str().unwrap_or("http"),
            uri.host().unwrap_or(""),
            uri.port_u16().unwrap_or_else(|| {
                match uri.scheme_str() {
                    Some("https") | Some("wss") => 443,
                    _ => 80,
                }
            })
        );

        self.semaphores_by_origin
            .entry(origin)
            .or_insert_with(|| Arc::new(Semaphore::new(self.max_connections_per_host)))
            .clone()
    }

    fn get_or_create_http_executor(
        &self,
        subgraph_name: &str,
        client_request: &ClientRequestDetails<'_>,
    ) -> Result<SubgraphExecutorBoxedArc, SubgraphExecutorError> {
        let endpoint_str = self.resolve_endpoint(subgraph_name, client_request)?;

        if let Some(executor) = self
            .http_executors_by_subgraph
            .get(subgraph_name)
            .and_then(|endpoints| endpoints.get(&endpoint_str).map(|e| e.clone()))
        {
            return Ok(executor);
        }

        self.register_executor(subgraph_name, &endpoint_str, None)
    }

    fn get_subscription_config(
        &self,
        subgraph_name: &str,
    ) -> &hive_router_config::subscriptions::SubscriptionsSubgraphConfig {
        self.config
            .subscriptions
            .subgraphs
            .get(subgraph_name)
            .unwrap_or(&self.config.subscriptions.all)
    }

    fn get_or_create_subscription_executor(
        &self,
        subgraph_name: &str,
        client_request: &ClientRequestDetails<'_>,
    ) -> Result<SubgraphExecutorBoxedArc, SubgraphExecutorError> {
        let subscription_config = self.get_subscription_config(subgraph_name);

        // the endpoint after applying any overrides or transformations
        let base_endpoint = self.resolve_endpoint(subgraph_name, client_request)?;

        // the subscription endpoint
        let endpoint_str = match subscription_config.protocol {
            SubscriptionProtocol::HTTP => base_endpoint.to_string(),
            SubscriptionProtocol::WebSocket => {
                let base_uri = base_endpoint.parse::<Uri>().map_err(|e| {
                    SubgraphExecutorError::EndpointParseFailure(
                        base_endpoint.to_string(),
                        e.to_string(),
                    )
                })?;

                let ws_scheme = match base_uri.scheme_str() {
                    Some("https") => "wss",
                    _ => "ws",
                };

                let path_and_query = subscription_config
                    .path
                    .as_deref()
                    .or_else(|| base_uri.path_and_query().map(|pq| pq.as_str()))
                    .unwrap_or("/");

                let endpoint_uri = Uri::builder()
                    .scheme(ws_scheme)
                    .authority(base_uri.authority().map(|a| a.as_str()).unwrap_or(""))
                    .path_and_query(path_and_query)
                    .build()
                    .map_err(|e| {
                        SubgraphExecutorError::EndpointParseFailure(
                            format!(
                                "{}://{}{}",
                                ws_scheme,
                                base_uri.authority().map(|a| a.as_str()).unwrap_or(""),
                                path_and_query
                            ),
                            e.to_string(),
                        )
                    })?;

                endpoint_uri.to_string()
            }
        };

        if let Some(executor) = self
            .subscription_executors_by_subgraph
            .get(subgraph_name)
            .and_then(|endpoints| endpoints.get(&endpoint_str).map(|e| e.clone()))
        {
            return Ok(executor);
        }

        self.register_executor(
            subgraph_name,
            &endpoint_str,
            Some(subscription_config.protocol),
        )
    }

    /// Registers a new HTTP subgraph executor for the given subgraph name and endpoint URL.
    /// It makes it availble for future requests.
    fn register_endpoint_expression(
        &mut self,
        subgraph_name: &str,
        expression: &str,
    ) -> Result<(), SubgraphExecutorError> {
        let program = expression.compile_expression(None).map_err(|err| {
            SubgraphExecutorError::EndpointExpressionBuild(
                subgraph_name.to_string(),
                err.diagnostics,
            )
        })?;
        self.expression_endpoints_by_subgraph
            .insert(subgraph_name.to_string(), program);

        Ok(())
    }

    /// Registers a static endpoint for the given subgraph name.
    /// This is used for quick lookup when no expression is defined
    /// or when resolving the expression (to have the original URL available there).
    fn register_static_endpoint(&self, subgraph_name: &str, endpoint_str: &str) {
        self.static_endpoints_by_subgraph
            .insert(subgraph_name.to_string(), endpoint_str.to_string());
    }

    /// Registers a subgraph executor for the given subgraph name and endpoint URL.
    /// If `subscription_protocol` is Some, creates the appropriate executor for that protocol
    /// and stores it in `subscription_executors_by_subgraph`.
    /// If `subscription_protocol` is None, creates an HTTP executor and stores it in `http_executors_by_subgraph`.
    fn register_executor(
        &self,
        subgraph_name: &str,
        endpoint_str: &str,
        subscription_protocol: Option<SubscriptionProtocol>,
    ) -> Result<SubgraphExecutorBoxedArc, SubgraphExecutorError> {
        if let Some(executor) = self
            .http_executors_by_subgraph
            .get(subgraph_name)
            .and_then(|endpoints| endpoints.get(endpoint_str).map(|e| e.clone()))
        {
            if let Some(protocol) = subscription_protocol {
                if matches!(protocol, SubscriptionProtocol::HTTP) {
                    self.subscription_executors_by_subgraph
                        .entry(subgraph_name.to_string())
                        .or_default()
                        .insert(endpoint_str.to_string(), executor.clone());
                }
            }
            return Ok(executor);
        }

        let endpoint_uri = endpoint_str.parse::<Uri>().map_err(|e| {
            SubgraphExecutorError::EndpointParseFailure(endpoint_str.to_string(), e.to_string())
        })?;

        let semaphore = self.get_semaphore_for_uri(&endpoint_uri);

        let executor_arc: SubgraphExecutorBoxedArc = match subscription_protocol {
            Some(SubscriptionProtocol::WebSocket) => {
                WsSubgraphExecutor::new(subgraph_name.to_string(), endpoint_uri, semaphore)
                    .to_boxed_arc()
            }
            _ => {
                let subgraph_config = self.resolve_subgraph_config(subgraph_name)?;
                HTTPSubgraphExecutor::new(
                    subgraph_name.to_string(),
                    endpoint_uri,
                    subgraph_config.client,
                    semaphore,
                    subgraph_config.dedupe_enabled,
                    self.in_flight_requests.clone(),
                )
                .to_boxed_arc()
            }
        };

        match subscription_protocol {
            Some(SubscriptionProtocol::HTTP) => {
                self.http_executors_by_subgraph
                    .entry(subgraph_name.to_string())
                    .or_default()
                    .insert(endpoint_str.to_string(), executor_arc.clone());
                self.subscription_executors_by_subgraph
                    .entry(subgraph_name.to_string())
                    .or_default()
                    .insert(endpoint_str.to_string(), executor_arc.clone());
            }
            Some(SubscriptionProtocol::WebSocket) => {
                self.subscription_executors_by_subgraph
                    .entry(subgraph_name.to_string())
                    .or_default()
                    .insert(endpoint_str.to_string(), executor_arc.clone());
            }
            None => {
                self.http_executors_by_subgraph
                    .entry(subgraph_name.to_string())
                    .or_default()
                    .insert(endpoint_str.to_string(), executor_arc.clone());
            }
        }

        Ok(executor_arc)
    }

    /// Resolves traffic shaping configuration for a specific subgraph, applying subgraph-specific
    /// overrides on top of global settings
    fn resolve_subgraph_config<'a>(
        &'a self,
        subgraph_name: &'a str,
    ) -> Result<ResolvedSubgraphConfig<'a>, SubgraphExecutorError> {
        let mut config = ResolvedSubgraphConfig {
            client: self.client.clone(),
            timeout_config: &self.config.traffic_shaping.all.request_timeout,
            dedupe_enabled: self.config.traffic_shaping.all.dedupe_enabled,
        };

        let Some(subgraph_config) = self.config.traffic_shaping.subgraphs.get(subgraph_name) else {
            return Ok(config);
        };

        // Override client only if pool idle timeout is customized
        if let Some(pool_idle_timeout) = subgraph_config.pool_idle_timeout {
            // Only override if it's different from the global setting
            if pool_idle_timeout != self.config.traffic_shaping.all.pool_idle_timeout {
                config.client = Arc::new(
                    Client::builder(TokioExecutor::new())
                        .pool_timer(TokioTimer::new())
                        .pool_idle_timeout(pool_idle_timeout)
                        .pool_max_idle_per_host(self.max_connections_per_host)
                        .build(build_https_executor()?),
                );
            }
        }

        // Apply other subgraph-specific overrides
        if let Some(dedupe_enabled) = subgraph_config.dedupe_enabled {
            config.dedupe_enabled = dedupe_enabled;
        }

        if let Some(custom_timeout) = &subgraph_config.request_timeout {
            config.timeout_config = custom_timeout;
        }

        Ok(config)
    }

    /// Compiles and registers a timeout for a specific subgraph.
    /// If the subgraph has a custom timeout configuration, it will be used.
    /// Otherwise, the global timeout configuration will be used.
    fn register_subgraph_timeout(&self, subgraph_name: &str) -> Result<(), SubgraphExecutorError> {
        // Check if this subgraph already has a timeout registered
        if self.timeouts_by_subgraph.contains_key(subgraph_name) {
            return Ok(());
        }

        // Get the timeout configuration for this subgraph, or fall back to global
        let timeout_config = self
            .config
            .traffic_shaping
            .subgraphs
            .get(subgraph_name)
            .and_then(|s| s.request_timeout.as_ref())
            .unwrap_or(&self.config.traffic_shaping.all.request_timeout);

        // Compile the timeout configuration into a DurationOrProgram
        let timeout_prog = DurationOrProgram::compile(timeout_config, None).map_err(|err| {
            SubgraphExecutorError::RequestTimeoutExpressionBuild(
                subgraph_name.to_string(),
                err.diagnostics,
            )
        })?;

        // Register the compiled timeout
        self.timeouts_by_subgraph
            .insert(subgraph_name.to_string(), timeout_prog);

        Ok(())
    }
}

/// Resolves a timeout DurationOrProgram to a concrete Duration.
/// Optionally includes a default timeout value in the VRL context.
fn resolve_timeout(
    duration_or_program: &DurationOrProgram,
    client_request: &ClientRequestDetails<'_>,
    default_timeout: Option<Duration>,
    timeout_name: &str,
) -> Result<Duration, SubgraphExecutorError> {
    duration_or_program
        .resolve(|| {
            let mut context_map = BTreeMap::new();
            context_map.insert("request".into(), client_request.into());

            if let Some(default) = default_timeout {
                context_map.insert(
                    "default".into(),
                    VrlValue::Integer(default.as_millis() as i64),
                );
            }

            VrlValue::Object(context_map)
        })
        .map_err(|err| {
            SubgraphExecutorError::TimeoutExpressionResolution(
                timeout_name.to_string(),
                err.to_string(),
            )
        })
}
