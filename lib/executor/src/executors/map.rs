use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};

use bytes::{BufMut, BytesMut};
use dashmap::DashMap;
use hive_router_config::{
    override_subgraph_urls::UrlOrExpression, traffic_shaping::DurationOrExpression,
    HiveRouterConfig,
};
use http::Uri;
use hyper_tls::HttpsConnector;
use hyper_util::{
    client::legacy::Client,
    rt::{TokioExecutor, TokioTimer},
};
use tokio::sync::{OnceCell, Semaphore};
use tracing::error;
use vrl::core::Value as VrlValue;

use crate::{
    execution::client_request_details::ClientRequestDetails,
    executors::{
        common::{
            HttpExecutionResponse, SubgraphExecutionRequest, SubgraphExecutor,
            SubgraphExecutorBoxedArc,
        },
        dedupe::{ABuildHasher, SharedResponse},
        error::SubgraphExecutorError,
        http::{HTTPSubgraphExecutor, HttpClient},
    },
    expressions::{CompileExpression, DurationOrProgram, StringOrProgram},
    response::graphql_error::GraphQLError,
};

type SubgraphName = String;
type EndpointOrProgram = StringOrProgram;
type ExecutorsBySubgraphMap = DashMap<SubgraphName, DashMap<String, SubgraphExecutorBoxedArc>>;
type EndpointsBySubgraphMap = DashMap<SubgraphName, EndpointOrProgram>;
type TimeoutsBySubgraph = DashMap<SubgraphName, DurationOrProgram>;

struct ResolvedSubgraphConfig<'a> {
    client: Arc<HttpClient>,
    timeout_config: &'a DurationOrExpression,
    dedupe_enabled: bool,
}

pub struct SubgraphExecutorMap {
    executors_by_subgraph: ExecutorsBySubgraphMap,
    /// Mapping from subgraph name to endpoint for quick lookup
    /// Can be either a static URL or a VRL expression that computes the endpoint
    endpoints_by_subgraph: EndpointsBySubgraphMap,
    timeouts_by_subgraph: TimeoutsBySubgraph,
    global_timeout: DurationOrProgram,
    config: Arc<HiveRouterConfig>,
    client: Arc<HttpClient>,
    semaphores_by_origin: DashMap<String, Arc<Semaphore>>,
    max_connections_per_host: usize,
    in_flight_requests: Arc<DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>>,
}

impl SubgraphExecutorMap {
    pub fn new(config: Arc<HiveRouterConfig>, global_timeout: DurationOrProgram) -> Self {
        let https = HttpsConnector::new();
        let client: HttpClient = Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(config.traffic_shaping.all.pool_idle_timeout)
            .pool_max_idle_per_host(config.traffic_shaping.max_connections_per_host)
            .build(https);

        let max_connections_per_host = config.traffic_shaping.max_connections_per_host;

        SubgraphExecutorMap {
            executors_by_subgraph: Default::default(),
            endpoints_by_subgraph: Default::default(),
            config,
            client: Arc::new(client),
            semaphores_by_origin: Default::default(),
            max_connections_per_host,
            in_flight_requests: Arc::new(DashMap::with_hasher(ABuildHasher::default())),
            timeouts_by_subgraph: Default::default(),
            global_timeout,
        }
    }

    pub fn from_http_endpoint_map(
        subgraph_endpoint_map: HashMap<SubgraphName, String>,
        config: Arc<HiveRouterConfig>,
    ) -> Result<Self, SubgraphExecutorError> {
        let global_timeout =
            DurationOrProgram::compile(&config.traffic_shaping.all.request_timeout, None).map_err(
                |err| {
                    SubgraphExecutorError::RequestTimeoutExpressionBuild(
                        "global".to_string(),
                        err.diagnostics,
                    )
                },
            )?;
        let subgraph_executor_map = SubgraphExecutorMap::new(config.clone(), global_timeout);

        for (subgraph_name, original_endpoint_str) in subgraph_endpoint_map.into_iter() {
            let endpoint_config = config
                .override_subgraph_urls
                .get_subgraph_url(&subgraph_name);

            let endpoint_or_prog = match endpoint_config {
                Some(UrlOrExpression::Url(url)) => StringOrProgram::Value(url.clone()),
                Some(UrlOrExpression::Expression { expression }) => {
                    let program = expression.compile_expression(None).map_err(|err| {
                        SubgraphExecutorError::EndpointExpressionBuild(
                            subgraph_name.to_string(),
                            err.diagnostics,
                        )
                    })?;
                    StringOrProgram::Program(Box::new(program))
                }
                None => StringOrProgram::Value(original_endpoint_str.clone()),
            };

            subgraph_executor_map
                .endpoints_by_subgraph
                .insert(subgraph_name.to_string(), endpoint_or_prog);

            subgraph_executor_map
                .register_executor(&subgraph_name, &original_endpoint_str)
                .ok();

            // Register the timeout configuration for this subgraph
            subgraph_executor_map.register_subgraph_timeout(&subgraph_name)?;
        }

        Ok(subgraph_executor_map)
    }

    pub async fn execute<'a, 'req>(
        &self,
        subgraph_name: &str,
        execution_request: SubgraphExecutionRequest<'a>,
        client_request: &ClientRequestDetails<'a, 'req>,
    ) -> HttpExecutionResponse {
        match self.get_or_create_executor(subgraph_name, client_request) {
            Ok(Some(executor)) => {
                let timeout = self
                    .timeouts_by_subgraph
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
                    .transpose();

                match timeout {
                    Ok(timeout) => executor.execute(execution_request, timeout).await,
                    Err(err) => {
                        error!(
                            "Failed to resolve timeout for subgraph '{}': {}",
                            subgraph_name, err,
                        );
                        self.internal_server_error_response(err.into(), subgraph_name)
                    }
                }
            }
            Err(err) => {
                error!(
                    "Subgraph executor error for subgraph '{}': {}",
                    subgraph_name, err,
                );
                self.internal_server_error_response(err.into(), subgraph_name)
            }
            Ok(None) => {
                error!(
                    "Subgraph executor not found for subgraph '{}'",
                    subgraph_name
                );
                self.internal_server_error_response("Internal server error".into(), subgraph_name)
            }
        }
    }

    fn internal_server_error_response(
        &self,
        graphql_error: GraphQLError,
        subgraph_name: &str,
    ) -> HttpExecutionResponse {
        let errors = vec![graphql_error.add_subgraph_name(subgraph_name)];
        let errors_bytes = sonic_rs::to_vec(&errors).unwrap();
        let mut buffer = BytesMut::new();
        buffer.put_slice(b"{\"errors\":");
        buffer.put_slice(&errors_bytes);
        buffer.put_slice(b"}");

        HttpExecutionResponse {
            body: buffer.freeze(),
            headers: Default::default(),
        }
    }

    /// Looks up or creates a subgraph executor based on the subgraph name.
    /// Resolves the endpoint URL (static or from expression) and creates an executor if needed.
    fn get_or_create_executor(
        &self,
        subgraph_name: &str,
        client_request: &ClientRequestDetails<'_, '_>,
    ) -> Result<Option<SubgraphExecutorBoxedArc>, SubgraphExecutorError> {
        // Resolve the endpoint URL (static or from expression)
        let endpoint_url = match self.resolve_endpoint(subgraph_name, client_request) {
            Ok(url) => url,
            Err(err) => {
                error!(
                    "Failed to resolve endpoint for subgraph '{}': {}",
                    subgraph_name, err
                );
                return Err(err);
            }
        };

        // Check if executor exists
        if let Some(executor) = self.get_executor_from_endpoint(subgraph_name, &endpoint_url) {
            return Ok(Some(executor));
        }

        // Create executor if it doesn't exist
        self.register_executor(subgraph_name, &endpoint_url)?;
        Ok(self.get_executor_from_endpoint(subgraph_name, &endpoint_url))
    }

    /// Resolves the endpoint URL for a subgraph, which can be either static or computed via expression
    fn resolve_endpoint(
        &self,
        subgraph_name: &str,
        client_request: &ClientRequestDetails<'_, '_>,
    ) -> Result<String, SubgraphExecutorError> {
        let endpoint_or_prog = self
            .endpoints_by_subgraph
            .get(subgraph_name)
            .ok_or_else(|| {
                SubgraphExecutorError::StaticEndpointNotFound(subgraph_name.to_string())
            })?;

        let context = VrlValue::Object(BTreeMap::from([("request".into(), client_request.into())]));

        endpoint_or_prog.value().resolve(context).map_err(|err| {
            SubgraphExecutorError::EndpointExpressionResolutionFailure(
                subgraph_name.to_string(),
                err.to_string(),
            )
        })
    }

    /// Looks up a subgraph executor for a given endpoint URL.
    /// Returns None if no executor exists for this endpoint.
    #[inline]
    fn get_executor_from_endpoint(
        &self,
        subgraph_name: &str,
        endpoint_str: &str,
    ) -> Option<SubgraphExecutorBoxedArc> {
        self.executors_by_subgraph
            .get(subgraph_name)
            .and_then(|endpoints| endpoints.get(endpoint_str).map(|e| e.clone()))
    }

    /// Registers a new HTTP subgraph executor for the given subgraph name and endpoint URL.
    /// It makes it available for future requests.
    fn register_executor(
        &self,
        subgraph_name: &str,
        endpoint_str: &str,
    ) -> Result<SubgraphExecutorBoxedArc, SubgraphExecutorError> {
        let endpoint_uri = endpoint_str.parse::<Uri>().map_err(|e| {
            SubgraphExecutorError::EndpointParseFailure(endpoint_str.to_string(), e.to_string())
        })?;

        let origin = format!(
            "{}://{}:{}",
            endpoint_uri.scheme_str().unwrap_or("http"),
            endpoint_uri.host().unwrap_or(""),
            endpoint_uri.port_u16().unwrap_or_else(|| {
                if endpoint_uri.scheme_str() == Some("https") {
                    443
                } else {
                    80
                }
            })
        );

        let semaphore = self
            .semaphores_by_origin
            .entry(origin)
            .or_insert_with(|| Arc::new(Semaphore::new(self.max_connections_per_host)))
            .clone();

        let subgraph_config = self.resolve_subgraph_config(subgraph_name);

        let executor = HTTPSubgraphExecutor::new(
            subgraph_name.to_string(),
            endpoint_uri,
            subgraph_config.client,
            semaphore,
            subgraph_config.dedupe_enabled,
            self.in_flight_requests.clone(),
        );

        let executor_arc = executor.to_boxed_arc();
        self.executors_by_subgraph
            .entry(subgraph_name.to_string())
            .or_default()
            .insert(endpoint_str.to_string(), executor_arc.clone());

        Ok(executor_arc)
    }

    /// Resolves traffic shaping configuration for a specific subgraph, applying subgraph-specific
    /// overrides on top of global settings
    fn resolve_subgraph_config<'a>(&'a self, subgraph_name: &'a str) -> ResolvedSubgraphConfig<'a> {
        let mut config = ResolvedSubgraphConfig {
            client: self.client.clone(),
            timeout_config: &self.config.traffic_shaping.all.request_timeout,
            dedupe_enabled: self.config.traffic_shaping.all.dedupe_enabled,
        };

        let Some(subgraph_config) = self.config.traffic_shaping.subgraphs.get(subgraph_name) else {
            return config;
        };

        // Override client only if pool idle timeout is customized
        if let Some(pool_idle_timeout) = subgraph_config.pool_idle_timeout {
            config.client = Arc::new(
                Client::builder(TokioExecutor::new())
                    .pool_timer(TokioTimer::new())
                    .pool_idle_timeout(pool_idle_timeout)
                    .pool_max_idle_per_host(self.max_connections_per_host)
                    .build(HttpsConnector::new()),
            );
        }

        // Apply other subgraph-specific overrides
        if let Some(dedupe_enabled) = subgraph_config.dedupe_enabled {
            config.dedupe_enabled = dedupe_enabled;
        }

        if let Some(custom_timeout) = &subgraph_config.request_timeout {
            config.timeout_config = custom_timeout;
        }

        config
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
    client_request: &ClientRequestDetails<'_, '_>,
    default_timeout: Option<Duration>,
    timeout_name: &str,
) -> Result<Duration, SubgraphExecutorError> {
    let mut context_map = BTreeMap::new();
    context_map.insert("request".into(), client_request.into());

    if let Some(default) = default_timeout {
        context_map.insert(
            "default".into(),
            VrlValue::Integer(default.as_millis() as i64),
        );
    }

    let context = VrlValue::Object(context_map);

    duration_or_program.resolve(context).map_err(|err| {
        SubgraphExecutorError::TimeoutExpressionResolution(
            timeout_name.to_string(),
            err.to_string(),
        )
    })
}
