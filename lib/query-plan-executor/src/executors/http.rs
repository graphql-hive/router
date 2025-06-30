use async_trait::async_trait;
use tracing::{error, instrument, trace};

use crate::{executors::common::SubgraphExecutor, ExecutionRequest, ExecutionResult};

#[derive(Debug)]
pub struct HTTPSubgraphExecutor {
    pub endpoint: String,
    pub http_client: reqwest::Client,
}

impl HTTPSubgraphExecutor {
    pub fn new(endpoint: String, http_client: reqwest::Client) -> Self {
        HTTPSubgraphExecutor {
            endpoint,
            http_client,
        }
    }

    async fn _execute(
        &self,
        execution_request: ExecutionRequest,
    ) -> Result<ExecutionResult, reqwest::Error> {
        trace!("Executing HTTP request to subgraph at {}", self.endpoint);

        let mut body = "{\"query\":".to_string()
            + &serde_json::to_string(&execution_request.query).unwrap()
            + ",\"variables\":{";
        let mut variables_added = false;
        if let Some(variables) = &execution_request.variables {
            let variables_entry = variables
                .iter()
                .map(|(key, value)| {
                    variables_added = true;
                    "\"".to_string() + key + "\": " + &serde_json::to_string(value).unwrap()
                })
                .collect::<Vec<String>>()
                .join(",");
            body.push_str(&variables_entry);
        }
        if let Some(representations) = &execution_request.representations {
            if variables_added {
                body.push(',');
            }
            body.push_str(&("\"representations\":".to_string() + representations));
        }
        body.push_str("}}");

        self.http_client
            .post(&self.endpoint)
            .body(body)
            .header("Content-Type", "application/json; charset=utf-8")
            .send()
            .await?
            .json::<ExecutionResult>()
            .await
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    #[instrument(level = "trace", skip(self), name = "http_subgraph_execute", fields(endpoint = %self.endpoint))]
    async fn execute(&self, execution_request: ExecutionRequest) -> ExecutionResult {
        self._execute(execution_request).await.unwrap_or_else(|e| {
            error!("Failed to execute request to subgraph: {}", e);
            trace!("network error: {:?}", e);

            ExecutionResult::from_error_message(format!(
                "Error executing subgraph {}: {}",
                self.endpoint, e
            ))
        })
    }
}
