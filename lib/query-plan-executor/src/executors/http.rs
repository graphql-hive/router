use async_trait::async_trait;
use tracing::{error, instrument, trace};

use crate::{executors::common::SubgraphExecutor, ExecutionRequest, ExecutionResult};

#[derive(Debug)]
pub struct HTTPSubgraphExecutor {
    pub endpoint: String,
    pub http_client: reqwest::Client,
}

const FIRST_VARIABLE_STR: &str = ",\"variables\":{";

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
    ) -> Result<ExecutionResult, String> {
        trace!("Executing HTTP request to subgraph at {}", self.endpoint);

        let mut body =
            "{\"query\":".to_string() + &serde_json::to_string(&execution_request.query).unwrap();
        let mut first_variable = true;
        if let Some(variables) = &execution_request.variables {
            for (variable_name, variable_value) in variables {
                if first_variable {
                    body.push_str(FIRST_VARIABLE_STR);
                    first_variable = false;
                } else {
                    body.push(',');
                }
                body.push('"');
                body.push_str(variable_name);
                body.push_str("\":");
                let value_str = serde_json::to_string(variable_value).map_err(|err| {
                    format!("Failed to serialize variable '{}': {}", variable_name, err)
                })?;
                body.push_str(&value_str);
            }
        }
        if let Some(representations) = &execution_request.representations {
            if first_variable {
                body.push_str(FIRST_VARIABLE_STR);
                first_variable = false;
            } else {
                body.push(',');
            }
            body.push_str("\"representations\":");
            body.push_str(representations);
        }
        // "first_variable" should be still true if there are no variables
        if !first_variable {
            body.push('}');
        }
        body.push('}');

        self.http_client
            .post(&self.endpoint)
            .body(body)
            .header("Content-Type", "application/json; charset=utf-8")
            .send()
            .await
            .map_err(|e| {
                format!(
                    "Failed to send request to subgraph {}: {}",
                    self.endpoint, e
                )
            })?
            .json::<ExecutionResult>()
            .await
            .map_err(|e| {
                format!(
                    "Failed to parse response from subgraph {}: {}",
                    self.endpoint, e
                )
            })
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    #[instrument(level = "trace", skip(self), name = "http_subgraph_execute", fields(endpoint = %self.endpoint))]
    async fn execute(&self, execution_request: ExecutionRequest) -> ExecutionResult {
        self._execute(execution_request).await.unwrap_or_else(|e| {
            error!(e);
            ExecutionResult::from_error_message(e)
        })
    }
}
