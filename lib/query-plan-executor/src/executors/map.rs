use std::{collections::HashMap, sync::Arc};

use tracing::{instrument, warn};

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

    #[instrument(level = "trace", name = "subgraph_execute", skip_all, fields(subgraph_name = %subgraph_name, execution_request = ?execution_request))]
    pub async fn execute<'a>(
        &self,
        subgraph_name: &str,
        execution_request: crate::SubgraphExecutionRequest<'a>,
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
        let http_client = cyper::Client::new();
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
