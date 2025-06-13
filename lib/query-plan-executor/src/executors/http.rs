use std::collections::HashMap;

use async_trait::async_trait;
use tracing::instrument;

use crate::{executors::common::SubgraphExecutor, ExecutionRequest, ExecutionResult};

#[derive(Debug)]
pub struct HTTPSubgraphExecutor<'a> {
    pub subgraph_endpoint_map: &'a HashMap<String, String>,
    pub http_client: &'a reqwest::Client,
}

impl HTTPSubgraphExecutor<'_> {
    async fn _execute(
        &self,
        subgraph_name: &str,
        execution_request: ExecutionRequest,
    ) -> Result<ExecutionResult, reqwest::Error> {
        match self.subgraph_endpoint_map.get(subgraph_name) {
            Some(subgraph_endpoint) => {
                self.http_client
                    .post(subgraph_endpoint)
                    .json(&execution_request)
                    .send()
                    .await?
                    .json::<ExecutionResult>()
                    .await
            }
            None => Ok(ExecutionResult::from_error_message(format!(
                "Subgraph {} not found in endpoint map",
                subgraph_name
            ))),
        }
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor<'_> {
    #[instrument(skip(self, execution_request))]
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: ExecutionRequest,
    ) -> ExecutionResult {
        self._execute(subgraph_name, execution_request)
            .await
            .unwrap_or_else(|e| {
                ExecutionResult::from_error_message(format!(
                    "Error executing subgraph {}: {}",
                    subgraph_name, e
                ))
            })
    }
}
