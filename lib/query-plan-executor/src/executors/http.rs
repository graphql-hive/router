use std::sync::Arc;

use async_trait::async_trait;
use tracing::instrument;

use crate::{executors::common::SubgraphExecutor, ExecutionRequest, ExecutionResult};

#[derive(Debug)]
pub struct HTTPSubgraphExecutor {
    pub endpoint: String,
    pub http_client: Arc<reqwest::Client>,
}

impl HTTPSubgraphExecutor {
    pub fn new(endpoint: String, http_client: Arc<reqwest::Client>) -> Self {
        HTTPSubgraphExecutor {
            endpoint,
            http_client,
        }
    }
    async fn _execute(
        &self,
        execution_request: ExecutionRequest,
    ) -> Result<ExecutionResult, reqwest::Error> {
        self.http_client
            .post(&self.endpoint)
            .json(&execution_request)
            .send()
            .await?
            .json::<ExecutionResult>()
            .await
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    #[instrument(skip(self, execution_request))]
    async fn execute(&self, execution_request: ExecutionRequest) -> ExecutionResult {
        self._execute(execution_request).await.unwrap_or_else(|e| {
            ExecutionResult::from_error_message(format!("Error executing subgraph {}", e))
        })
    }
}
