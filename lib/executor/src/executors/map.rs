use std::{collections::HashMap, sync::Arc, time::Duration};

use bytes::{BufMut, Bytes, BytesMut};
use dashmap::DashMap;
use http::Uri;
use hyper_util::{
    client::legacy::Client,
    rt::{TokioExecutor, TokioTimer},
};
use router_config::traffic_shaping::TrafficShapingExecutorConfig;
use tokio::sync::{OnceCell, Semaphore};

use crate::{
    executors::{
        common::{HttpExecutionRequest, SubgraphExecutor, SubgraphExecutorBoxedArc},
        dedupe::{ABuildHasher, RequestFingerprint, SharedResponse},
        error::SubgraphExecutorError,
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
    ) -> Bytes {
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
                buffer.freeze()
            }
        }
    }

    pub fn insert_boxed_arc(&mut self, subgraph_name: String, boxed_arc: SubgraphExecutorBoxedArc) {
        self.inner.insert(subgraph_name, boxed_arc);
    }

    pub fn from_http_endpoint_map(
        subgraph_endpoint_map: HashMap<String, String>,
        config: TrafficShapingExecutorConfig,
    ) -> Result<Self, SubgraphExecutorError> {
        let client = Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(Duration::from_secs(config.pool_idle_timeout_seconds))
            .pool_max_idle_per_host(config.max_connections_per_host)
            .build_http();

        let client_arc = Arc::new(client);
        let semaphores_by_origin: DashMap<String, Arc<Semaphore>> = DashMap::new();
        let max_connections_per_host = config.max_connections_per_host;
        let config_arc = Arc::new(config);
        let in_flight_requests: Arc<
            DashMap<RequestFingerprint, Arc<OnceCell<SharedResponse>>, ABuildHasher>,
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

                let semaphore = semaphores_by_origin
                    .entry(origin)
                    .or_insert_with(|| Arc::new(Semaphore::new(max_connections_per_host)))
                    .clone();

                let executor = HTTPSubgraphExecutor::new(
                    endpoint_uri,
                    client_arc.clone(),
                    semaphore,
                    config_arc.clone(),
                    in_flight_requests.clone(),
                );

                Ok((subgraph_name, executor.to_boxed_arc()))
            })
            .collect::<Result<HashMap<_, _>, SubgraphExecutorError>>()?;

        Ok(SubgraphExecutorMap {
            inner: executor_map,
        })
    }
}
