use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::header::CONTENT_TYPE;
use tracing::{error, instrument, trace};

use crate::{executors::common::SubgraphExecutor, ExecutionRequest, ExecutionResult};

pub struct HTTPSubgraphExecutor {
    pub subgraph_endpoint_map: HashMap<String, String>,
    http_client: reqwest::Client,
}

impl HTTPSubgraphExecutor {
    pub fn new(subgraph_endpoint_map: HashMap<String, String>) -> Self {
        HTTPSubgraphExecutor {
            subgraph_endpoint_map,
            http_client: reqwest::Client::new(),
        }
    }

    async fn _execute(
        &self,
        subgraph_name: &str,
        execution_request: ExecutionRequest,
    ) -> Result<ExecutionResult, reqwest::Error> {
        match self.subgraph_endpoint_map.get(subgraph_name) {
            Some(subgraph_endpoint) => {
                let request_body_bytes =
                    sonic_rs::to_vec(&execution_request).expect("to JSON(body)");
                let response = self
                    .http_client
                    .post(subgraph_endpoint)
                    .header(CONTENT_TYPE, "application/json")
                    .body(request_body_bytes)
                    .send()
                    .await?;

                let response_bytes = response.bytes().await?;

                let execution_result = sonic_rs::from_slice::<ExecutionResult>(&response_bytes)
                    .expect("parse(response)");

                Ok(execution_result)
            }
            None => Ok(ExecutionResult::from_error_message(format!(
                "Subgraph {} not found in endpoint map",
                subgraph_name
            ))),
        }
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    #[instrument(
        level = "trace",
        skip(self, execution_request),
        name = "HTTPSubgraphExecutor"
    )]
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: ExecutionRequest,
    ) -> ExecutionResult {
        self._execute(subgraph_name, execution_request)
            .await
            .unwrap_or_else(|e| {
                error!("Failed to execute request to subgraph: {}", e);
                trace!("network error: {:?}", e);

                ExecutionResult::from_error_message(format!(
                    "Error executing subgraph {}: {}",
                    subgraph_name, e
                ))
            })
    }
}
