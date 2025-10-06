use std::{collections::HashMap, sync::Arc, time::Duration};

use bytes::{BufMut, BytesMut};
use dashmap::DashMap;
use hive_router_config::{
    override_subgraph_urls::{OverrideSubgraphUrlConfig, OverrideSubgraphUrlsConfig},
    traffic_shaping::TrafficShapingExecutorConfig,
};
use hyper_tls::HttpsConnector;
use hyper_util::{
    client::legacy::Client,
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
        expression_http::ExpressionHTTPExecutor,
        http::HTTPSubgraphExecutor,
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
        override_subgraph_urls_config: OverrideSubgraphUrlsConfig,
        traffic_shaping_config: TrafficShapingExecutorConfig,
    ) -> Result<Self, SubgraphExecutorError> {
        let https = HttpsConnector::new();
        let client = Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(Duration::from_secs(
                traffic_shaping_config.pool_idle_timeout_seconds,
            ))
            .pool_max_idle_per_host(traffic_shaping_config.max_connections_per_host)
            .build(https);

        let client_arc = Arc::new(client);
        let semaphores_by_origin: DashMap<String, Arc<Semaphore>> = DashMap::new();
        let semaphores_by_origin_arc = Arc::new(semaphores_by_origin);
        let config_arc = Arc::new(traffic_shaping_config);
        let in_flight_requests: Arc<DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>> =
            Arc::new(DashMap::with_hasher(ABuildHasher::default()));

        let executor_map = subgraph_endpoint_map
            .into_iter()
            .map(|(subgraph_name, endpoint_str)| {
                let executor: Arc<Box<dyn SubgraphExecutor + Send + Sync>> =
                    if let Some(subgraph_entry) =
                        override_subgraph_urls_config.subgraphs.get(&subgraph_name)
                    {
                        match subgraph_entry {
                            OverrideSubgraphUrlConfig::Url(endpoint_str) => {
                                HTTPSubgraphExecutor::try_new(
                                    endpoint_str,
                                    client_arc.clone(),
                                    semaphores_by_origin_arc.clone(),
                                    config_arc.clone(),
                                    in_flight_requests.clone(),
                                )?
                                .to_boxed_arc()
                            }
                            OverrideSubgraphUrlConfig::Expression(expression) => {
                                ExpressionHTTPExecutor::try_new(
                                    &endpoint_str,
                                    expression,
                                    client_arc.clone(),
                                    config_arc.clone(),
                                    in_flight_requests.clone(),
                                    semaphores_by_origin_arc.clone(),
                                )?
                                .to_boxed_arc()
                            }
                        }
                    } else {
                        HTTPSubgraphExecutor::try_new(
                            &endpoint_str,
                            client_arc.clone(),
                            semaphores_by_origin_arc.clone(),
                            config_arc.clone(),
                            in_flight_requests.clone(),
                        )?
                        .to_boxed_arc()
                    };

                Ok((subgraph_name, executor))
            })
            .collect::<Result<HashMap<_, _>, SubgraphExecutorError>>()?;

        Ok(SubgraphExecutorMap {
            inner: executor_map,
        })
    }
}
