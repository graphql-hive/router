use std::sync::Arc;

use async_graphql::{Executor, Request, Response, ServerError};
use futures::stream::BoxStream;
use tracing::instrument;

#[derive(Debug, Clone)]
pub struct HTTPSubgraphExecutor {
    pub endpoint: String,
    pub http_client: Arc<reqwest::Client>,
}

impl HTTPSubgraphExecutor {
    pub fn new(endpoint: &str, http_client: Arc<reqwest::Client>) -> Self {
        HTTPSubgraphExecutor {
            endpoint: endpoint.to_string(),
            http_client,
        }
    }
    async fn _execute(
        &self,
        execution_request: Request,
    ) -> Result<Response, reqwest::Error> {
        self.http_client
            .post(&self.endpoint)
            .json(&execution_request)
            .send()
            .await?
            .json::<Response>()
            .await
    }
}

impl Executor for HTTPSubgraphExecutor {
    #[instrument(skip(self, execution_request))]
    async fn execute(&self, execution_request: Request) -> Response {
        self._execute(execution_request).await.unwrap_or_else(|e| {
            let error_message = format!(
                "Error executing subgraph at endpoint {}: {}",
                self.endpoint, e
            );
            Response::from_errors(
                vec![
                    ServerError::new(error_message, None)
                ]
            )   
        })
    }

    fn execute_stream(
        &self,
        _request: Request,
        _session_data: Option<Arc<async_graphql::Data>>,
    ) -> BoxStream<'static, Response> {
        unimplemented!("HTTPSubgraphExecutor does not support streaming execution yet.")
    }
}
