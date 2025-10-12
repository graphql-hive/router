use std::{collections::HashMap, sync::Arc, time::Duration};

use bytes::{BufMut, Bytes, BytesMut};
use dashmap::DashMap;
use hive_router_config::{traffic_shaping::TrafficShapingExecutorConfig, TrafficShapingConfig};
use http::Uri;
use http_body_util::Full;
use hyper_tls::HttpsConnector;
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::{TokioExecutor, TokioTimer},
};
use tokio::sync::{OnceCell, Semaphore};

use crate::{
    executors::{
        common::{
            HttpExecutionRequest, HttpExecutionResponse, SubgraphExecutor, SubgraphExecutorBoxedArc,
        },
        dedupe::{ABuildHasher, SharedResponse},
        error::SubgraphExecutorError,
        http::HTTPSubgraphExecutor,
        timeout::TimeoutExecutor,
    },
    response::graphql_error::GraphQLError,
};

pub struct SubgraphExecutorMap {
    inner: HashMap<String, SubgraphExecutorBoxedArc>,
}

impl Default for SubgraphExecutorMap {
    fn default() -> Self {
        Self::new()
    }
}

impl SubgraphExecutorMap {
    pub fn new() -> Self {
        SubgraphExecutorMap {
            inner: HashMap::new(),
        }
    }

    pub async fn execute<'a>(
        &self,
        subgraph_name: &str,
        execution_request: HttpExecutionRequest<'a>,
    ) -> HttpExecutionResponse {
        match self.inner.get(subgraph_name) {
            Some(executor) => executor.execute(execution_request).await,
            None => {
                let graphql_error: GraphQLError = format!(
                    "Subgraph executor not found for subgraph: {}",
                    subgraph_name
                )
                .into();
                let errors = vec![graphql_error];
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
        }
    }

    pub fn insert_boxed_arc(&mut self, subgraph_name: String, boxed_arc: SubgraphExecutorBoxedArc) {
        self.inner.insert(subgraph_name, boxed_arc);
    }

    pub fn from_http_endpoint_map(
        subgraph_endpoint_map: HashMap<String, String>,
        config: TrafficShapingConfig,
    ) -> Result<Self, SubgraphExecutorError> {
        let global_client_arc = from_traffic_shaping_config_to_client(&config.all);
        let global_semaphores_by_origin: DashMap<String, Arc<Semaphore>> = DashMap::new();
        let global_config_arc = Arc::new(config.all);
        let global_in_flight_requests: Arc<
            DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>,
        > = Arc::new(DashMap::with_hasher(ABuildHasher::default()));

        let executor_map = subgraph_endpoint_map
            .into_iter()
            .map(|(subgraph_name, endpoint_str)| {
                let endpoint_uri = endpoint_str.parse::<Uri>().map_err(|e| {
                    SubgraphExecutorError::EndpointParseFailure(endpoint_str.clone(), e.to_string())
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

                let subgraph_config = config.subgraphs.get(&subgraph_name);

                let semaphore = get_semaphore_for_subgraph(
                    &origin,
                    &global_semaphores_by_origin,
                    subgraph_config
                        .map(|cfg| cfg.max_connections_per_host)
                        .unwrap_or(global_config_arc.max_connections_per_host),
                    global_config_arc.max_connections_per_host,
                );

                let http_client = get_http_client_for_subgraph(
                    subgraph_config,
                    &global_config_arc,
                    &global_client_arc,
                );

                // TODO: Maybe reuse the in-flight requests map in some cases ???
                let inflight_requests = subgraph_config
                    .map(|_| Arc::new(DashMap::with_hasher(ABuildHasher::default())))
                    .unwrap_or_else(|| global_in_flight_requests.clone());

                let config_arc = subgraph_config
                    .map(|cfg| Arc::new(cfg.clone()))
                    .unwrap_or_else(|| global_config_arc.clone());

                let mut executor = HTTPSubgraphExecutor::new(
                    endpoint_uri.clone(),
                    http_client,
                    semaphore,
                    config_arc.clone(),
                    inflight_requests,
                )
                .to_boxed_arc();

                if let Some(timeout_config) = &config_arc.timeout {
                    executor = TimeoutExecutor::try_new(endpoint_uri, timeout_config, executor)?
                        .to_boxed_arc();
                }

                Ok((subgraph_name, executor))
            })
            .collect::<Result<HashMap<_, _>, SubgraphExecutorError>>()?;

        Ok(SubgraphExecutorMap {
            inner: executor_map,
        })
    }
}

// Create a new hyper client based on the traffic shaping config
fn from_traffic_shaping_config_to_client(
    config: &TrafficShapingExecutorConfig,
) -> Arc<Client<HttpsConnector<HttpConnector>, Full<Bytes>>> {
    Arc::new(
        Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(Duration::from_secs(config.pool_idle_timeout_seconds))
            .pool_max_idle_per_host(config.max_connections_per_host)
            .build(HttpsConnector::new()),
    )
}

// Reuse the global client if the subgraph config is the same as the global config
// Otherwise, create a new client based on the subgraph config
fn get_http_client_for_subgraph(
    subgraph_config: Option<&TrafficShapingExecutorConfig>,
    global_config: &TrafficShapingExecutorConfig,
    global_client: &Arc<Client<HttpsConnector<HttpConnector>, Full<Bytes>>>,
) -> Arc<Client<HttpsConnector<HttpConnector>, Full<Bytes>>> {
    match subgraph_config {
        Some(cfg) => {
            if global_config.max_connections_per_host == cfg.max_connections_per_host
                && global_config.pool_idle_timeout_seconds == cfg.pool_idle_timeout_seconds
            {
                global_client.clone()
            } else {
                from_traffic_shaping_config_to_client(cfg)
            }
        }
        None => global_client.clone(),
    }
}

// If the subgraph has a specific max_connections_per_host, create a new semaphore for it.
// Otherwise, reuse the global semaphore for that origin.
fn get_semaphore_for_subgraph(
    origin: &str,
    semaphores_by_origin: &DashMap<String, Arc<Semaphore>>,
    max_connections_per_host: usize,
    global_max_connections_per_host: usize,
) -> Arc<Semaphore> {
    if max_connections_per_host == global_max_connections_per_host {
        semaphores_by_origin
            .entry(origin.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(global_max_connections_per_host)))
            .clone()
    } else {
        Arc::new(Semaphore::new(max_connections_per_host))
    }
}
