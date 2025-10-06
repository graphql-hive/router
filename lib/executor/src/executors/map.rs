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
    pub inner: HashMap<String, SubgraphExecutorBoxedArc>,
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
                            OverrideSubgraphUrlConfig::Url { url: endpoint_str } => {
                                HTTPSubgraphExecutor::try_new(
                                    endpoint_str,
                                    client_arc.clone(),
                                    semaphores_by_origin_arc.clone(),
                                    config_arc.clone(),
                                    in_flight_requests.clone(),
                                )?
                                .to_boxed_arc()
                            }
                            OverrideSubgraphUrlConfig::Expression { expression } => {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use hive_router_config::parse_yaml_config;

    use crate::{
        executors::{expression_http::ExpressionHTTPExecutor, http::HTTPSubgraphExecutor},
        SubgraphExecutorMap,
    };

    #[test]
    fn create_executors_with_override_subgraph_urls() {
        let yaml_str = r#"
        override_subgraph_urls:
            subgraphs:
                subgraph_a:
                    url: http://subgraph-a-new.com/graphql
                subgraph_b:
                    expression: |
                        if .request.headers."x-region" == "eu-west" {
                            "http://subgraph-b-eu-west.com/graphql"
                        } else {
                            "http://subgraph-b.com/graphql"
                        }
        "#;
        let mut subgraph_endpoint_map = HashMap::new();
        subgraph_endpoint_map.insert(
            "subgraph_a".to_string(),
            "http://subgraph-a.com/graphql".to_string(),
        );
        subgraph_endpoint_map.insert(
            "subgraph_b".to_string(),
            "http://subgraph-b-eu-west.com/graphql".to_string(),
        );
        subgraph_endpoint_map.insert(
            "subgraph_c".to_string(),
            "http://subgraph-c.com/graphql".to_string(),
        );
        let config = parse_yaml_config(String::from(yaml_str)).unwrap();
        let executor_map = SubgraphExecutorMap::from_http_endpoint_map(
            subgraph_endpoint_map,
            config.override_subgraph_urls,
            config.traffic_shaping,
        )
        .unwrap();
        assert_eq!(executor_map.inner.len(), 3);
        let executor_a = executor_map.inner.get("subgraph_a").unwrap();
        let http_executor_a = executor_a
            .as_any()
            .downcast_ref::<HTTPSubgraphExecutor>()
            .unwrap();
        // cast to HTTPSubgraphExecutor
        assert_eq!(
            http_executor_a.endpoint.to_string(),
            "http://subgraph-a-new.com/graphql"
        );
        let executor_b = executor_map.inner.get("subgraph_b").unwrap();
        let expr_executor_b = executor_b
            .as_any()
            .downcast_ref::<ExpressionHTTPExecutor>()
            .unwrap();
        assert_eq!(
            expr_executor_b.default_endpoint.to_string(),
            "http://subgraph-b-eu-west.com/graphql"
        );
        let executor_c = executor_map.inner.get("subgraph_c").unwrap();
        let http_executor_c = executor_c
            .as_any()
            .downcast_ref::<HTTPSubgraphExecutor>()
            .unwrap();
        assert_eq!(
            http_executor_c.endpoint.to_string(),
            "http://subgraph-c.com/graphql"
        );
    }
}
