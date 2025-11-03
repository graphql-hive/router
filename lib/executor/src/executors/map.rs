use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use bytes::{BufMut, Bytes, BytesMut};
use dashmap::DashMap;
use hive_router_config::{override_subgraph_urls::UrlOrExpression, HiveRouterConfig};
use http::Uri;
use hyper_tls::HttpsConnector;
use hyper_util::{
    client::legacy::Client,
    rt::{TokioExecutor, TokioTimer},
};
use tokio::sync::{OnceCell, Semaphore};
use tracing::error;
use vrl::{compiler::Program as VrlProgram, core::Value as VrlValue};

use crate::{
    execution::client_request_details::ClientRequestDetails,
    executors::{
        common::{
            HttpExecutionRequest, HttpExecutionResponse, SubgraphExecutor, SubgraphExecutorBoxedArc,
        },
        dedupe::{ABuildHasher, SharedResponse},
        error::SubgraphExecutorError,
        http::{HTTPSubgraphExecutor, HttpClient},
    },
    response::graphql_error::GraphQLError,
    utils::expression::{compile_expression, execute_expression_with_value},
};

type SubgraphName = String;
type SubgraphEndpoint = String;
type ExecutorsBySubgraphMap =
    DashMap<SubgraphName, DashMap<SubgraphEndpoint, SubgraphExecutorBoxedArc>>;
type EndpointsBySubgraphMap = DashMap<SubgraphName, SubgraphEndpoint>;
type ExpressionsBySubgraphMap = HashMap<SubgraphName, VrlProgram>;

pub struct SubgraphExecutorMap {
    executors_by_subgraph: ExecutorsBySubgraphMap,
    /// Mapping from subgraph name to endpoint for quick lookup
    /// based on supergrah sdl and static overrides from router's config.
    static_endpoints_by_subgraph: EndpointsBySubgraphMap,
    /// Mapping from subgraph name to VRL expression program
    expressions_by_subgraph: ExpressionsBySubgraphMap,
    config: Arc<HiveRouterConfig>,
    client: Arc<HttpClient>,
    semaphores_by_origin: DashMap<String, Arc<Semaphore>>,
    max_connections_per_host: usize,
    in_flight_requests: Arc<DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>>,
}

impl SubgraphExecutorMap {
    pub fn new(config: Arc<HiveRouterConfig>) -> Self {
        let https = HttpsConnector::new();
        let client: HttpClient = Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(config.traffic_shaping.pool_idle_timeout)
            .pool_max_idle_per_host(config.traffic_shaping.max_connections_per_host)
            .build(https);

        let max_connections_per_host = config.traffic_shaping.max_connections_per_host;

        SubgraphExecutorMap {
            executors_by_subgraph: Default::default(),
            static_endpoints_by_subgraph: Default::default(),
            expressions_by_subgraph: Default::default(),
            config,
            client: Arc::new(client),
            semaphores_by_origin: Default::default(),
            max_connections_per_host,
            in_flight_requests: Arc::new(DashMap::with_hasher(ABuildHasher::default())),
        }
    }

    pub fn from_http_endpoint_map(
        subgraph_endpoint_map: HashMap<SubgraphName, SubgraphEndpoint>,
        config: Arc<HiveRouterConfig>,
    ) -> Result<Self, SubgraphExecutorError> {
        let mut subgraph_executor_map = SubgraphExecutorMap::new(config.clone());

        for (subgraph_name, original_endpoint_str) in subgraph_endpoint_map.into_iter() {
            let endpoint_str = config
                .override_subgraph_urls
                .get_subgraph_url(&subgraph_name);

            let endpoint_str = match endpoint_str {
                Some(UrlOrExpression::Url(url)) => url,
                Some(UrlOrExpression::Expression { expression }) => {
                    subgraph_executor_map.register_expression(&subgraph_name, expression)?;
                    &original_endpoint_str
                }
                None => &original_endpoint_str,
            };

            subgraph_executor_map.register_executor(&subgraph_name, endpoint_str)?;
            subgraph_executor_map.register_static_endpoint(&subgraph_name, endpoint_str);
        }

        Ok(subgraph_executor_map)
    }

    pub async fn execute<'a, 'req>(
        &self,
        subgraph_name: &str,
        execution_request: HttpExecutionRequest<'a>,
        client_request: &ClientRequestDetails<'a, 'req>,
    ) -> HttpExecutionResponse {
        match self.get_or_create_executor(subgraph_name, client_request) {
            Ok(Some(executor)) => executor.execute(execution_request).await,
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

    /// Looks up a subgraph executor based on the subgraph name.
    /// Looks for an expression first, falling back to a static endpoint.
    /// If nothing is found, returns None.
    fn get_or_create_executor(
        &self,
        subgraph_name: &str,
        client_request: &ClientRequestDetails<'_, '_>,
    ) -> Result<Option<SubgraphExecutorBoxedArc>, SubgraphExecutorError> {
        let from_expression =
            self.get_or_create_executor_from_expression(subgraph_name, client_request)?;

        if from_expression.is_some() {
            return Ok(from_expression);
        }

        Ok(self.get_executor_from_static_endpoint(subgraph_name))
    }

    /// Looks up a subgraph executor,
    /// or creates one if a VRL expression is defined for the subgraph.
    /// The expression is resolved to get the endpoint URL,
    /// and a new executor is created and stored for future requests.
    fn get_or_create_executor_from_expression(
        &self,
        subgraph_name: &str,
        client_request: &ClientRequestDetails<'_, '_>,
    ) -> Result<Option<SubgraphExecutorBoxedArc>, SubgraphExecutorError> {
        if let Some(expression) = self.expressions_by_subgraph.get(subgraph_name) {
            let original_url_value = VrlValue::Bytes(Bytes::from(
                self.static_endpoints_by_subgraph
                    .get(subgraph_name)
                    .map(|endpoint| endpoint.value().clone())
                    .ok_or_else(|| {
                        SubgraphExecutorError::StaticEndpointNotFound(subgraph_name.to_string())
                    })?,
            ));
            let value = VrlValue::Object(BTreeMap::from([
                ("request".into(), client_request.into()),
                ("original_url".into(), original_url_value),
            ]));

            // Resolve the expression to get an endpoint URL.
            let endpoint_result =
                execute_expression_with_value(expression, value).map_err(|err| {
                    SubgraphExecutorError::new_endpoint_expression_resolution_failure(
                        subgraph_name.to_string(),
                        err,
                    )
                })?;
            let endpoint_str = match endpoint_result.as_str() {
                Some(s) => s.to_string(),
                None => {
                    return Err(SubgraphExecutorError::EndpointExpressionWrongType(
                        subgraph_name.to_string(),
                    ));
                }
            };

            // Check if an executor for this endpoint already exists.
            let existing_executor = self
                .executors_by_subgraph
                .get(subgraph_name)
                .and_then(|endpoints| endpoints.get(&endpoint_str).map(|e| e.clone()));

            if let Some(executor) = existing_executor {
                return Ok(Some(executor));
            }

            // If not, create and register a new one.
            self.register_executor(subgraph_name, &endpoint_str)?;

            let endpoints = self
                .executors_by_subgraph
                .get(subgraph_name)
                .expect("Executor was just registered, should be present");
            return Ok(endpoints.get(&endpoint_str).map(|e| e.clone()));
        }

        Ok(None)
    }

    /// Looks up a subgraph executor based on a static endpoint URL.
    fn get_executor_from_static_endpoint(
        &self,
        subgraph_name: &str,
    ) -> Option<SubgraphExecutorBoxedArc> {
        self.static_endpoints_by_subgraph
            .get(subgraph_name)
            .and_then(|endpoint_ref| {
                let endpoint_str = endpoint_ref.value();
                self.executors_by_subgraph
                    .get(subgraph_name)
                    .and_then(|endpoints| endpoints.get(endpoint_str).map(|e| e.clone()))
            })
    }

    /// Registers a VRL expression for the given subgraph name.
    /// The expression can later be used to resolve the endpoint URL cheaply,
    /// without needing to recompile it every time.
    fn register_expression(
        &mut self,
        subgraph_name: &str,
        expression: &str,
    ) -> Result<(), SubgraphExecutorError> {
        let program = compile_expression(expression, None).map_err(|err| {
            SubgraphExecutorError::EndpointExpressionBuild(subgraph_name.to_string(), err)
        })?;
        self.expressions_by_subgraph
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

    /// Registers a new HTTP subgraph executor for the given subgraph name and endpoint URL.
    /// It makes it availble for future requests.
    fn register_executor(
        &self,
        subgraph_name: &str,
        endpoint_str: &str,
    ) -> Result<(), SubgraphExecutorError> {
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

        let executor = HTTPSubgraphExecutor::new(
            subgraph_name.to_string(),
            endpoint_uri,
            self.client.clone(),
            semaphore,
            self.config.clone(),
            self.in_flight_requests.clone(),
        );

        self.executors_by_subgraph
            .entry(subgraph_name.to_string())
            .or_default()
            .insert(endpoint_str.to_string(), executor.to_boxed_arc());

        Ok(())
    }
}
