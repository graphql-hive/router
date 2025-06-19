use std::collections::HashMap;

use tracing::{instrument, warn};

use crate::executors::common::{SubgraphExecutor, SubgraphExecutorBoxedArc};

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

    #[instrument(level = "trace", name = "subgraph_execute", skip_all)]
    pub async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: crate::ExecutionRequest,
    ) -> crate::ExecutionResult {
        match self.inner.get(subgraph_name) {
            Some(executor) => executor.execute(execution_request).await,
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
        let http_client = reqwest::Client::new();
        let executor_map = subgraph_endpoint_map
            .into_iter()
            .map(|(subgraph_name, endpoint)| {
                let executor = crate::executors::http::HTTPSubgraphExecutor::new(
                    endpoint,
                    http_client.clone(),
                )
                .to_boxed_arc();
                (subgraph_name, executor)
            })
            .collect::<HashMap<_, _>>();
        SubgraphExecutorMap {
            inner: executor_map,
        }
    }
}
