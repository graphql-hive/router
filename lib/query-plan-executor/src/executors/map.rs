use std::{collections::HashMap, sync::Arc, time::Duration};

use hyper_util::{
    client::legacy::Client,
    rt::{TokioExecutor, TokioTimer},
};
use tracing::warn;

use crate::executors::{
    common::{SubgraphExecutor, SubgraphExecutorBoxedArc},
    http::HTTPSubgraphExecutor,
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
        execution_request: crate::SubgraphExecutionRequest<'a>,
    ) -> crate::ExecutionResult {
        match self.inner.get(subgraph_name) {
            Some(executor) => {
                let mut result = executor.execute(execution_request).await;
                if let Some(errors) = &mut result.errors {
                    for error in errors {
                        match error.extensions {
                            Some(ref mut extensions) => {
                                if extensions.get("code").is_none() {
                                    extensions.insert(
                                        "code".to_string(),
                                        serde_json::Value::String(
                                            "DOWNSTREAM_SERVICE_ERROR".to_string(),
                                        ),
                                    );
                                }
                                if extensions.get("serviceName").is_none() {
                                    extensions.insert(
                                        "serviceName".to_string(),
                                        serde_json::Value::String(subgraph_name.to_string()),
                                    );
                                }
                            }
                            None => {
                                error.extensions = Some(HashMap::from([
                                    (
                                        "code".to_string(),
                                        serde_json::Value::String(
                                            "DOWNSTREAM_SERVICE_ERROR".to_string(),
                                        ),
                                    ),
                                    (
                                        "serviceName".to_string(),
                                        serde_json::Value::String(subgraph_name.to_string()),
                                    ),
                                ]));
                            }
                        }
                    }
                }
                result
            }
            None => {
                warn!(
                    "Subgraph executor not found for subgraph: {}",
                    subgraph_name
                );
                crate::ExecutionResult::from_error_message(format!(
                    "Subgraph executor not found for subgraph: {}",
                    subgraph_name
                ))
            }
        }
    }

    pub fn insert_boxed_arc(&mut self, subgraph_name: String, boxed_arc: SubgraphExecutorBoxedArc) {
        self.inner.insert(subgraph_name, boxed_arc);
    }

    pub fn from_http_endpoint_map(subgraph_endpoint_map: HashMap<String, String>) -> Self {
        let mut builder = Client::builder(TokioExecutor::new());
        let builder_mut = builder
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(Duration::from_secs(60 * 60))
            .pool_max_idle_per_host(usize::MAX);
        let http_client = builder_mut.build_http();
        let http_client_arc = Arc::new(http_client);
        let executor_map = subgraph_endpoint_map
            .into_iter()
            .map(|(subgraph_name, endpoint)| {
                let executor =
                    HTTPSubgraphExecutor::new(&endpoint, http_client_arc.clone()).to_boxed_arc();
                (subgraph_name, executor)
            })
            .collect::<HashMap<_, _>>();
        SubgraphExecutorMap {
            inner: executor_map,
        }
    }
}
