use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures_util::StreamExt;
use ntex::rt::Arbiter;
use tracing::debug;

use crate::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use crate::executors::error::SubgraphExecutorError;
use crate::executors::graphql_transport_ws::ConnectionInitPayload;
use crate::executors::websocket_client::{connect, WsClient};
use crate::response::subgraph_response::SubgraphResponse;

pub struct WsSubgraphExecutor {
    subgraph_name: String,
    endpoint: String,
    arbiter: Arbiter,
}

impl WsSubgraphExecutor {
    pub fn new(subgraph_name: String, endpoint: String) -> Self {
        Self {
            subgraph_name,
            endpoint,
            // each executors its own arbiter because the WsClient is not sync+send compatible,
            // so we instead spawn a dedicated arbiter (maps to one OS thread) to run all websocket
            // connection tasks for this executor
            arbiter: Arbiter::new(),
        }
    }
}

impl Drop for WsSubgraphExecutor {
    fn drop(&mut self) {
        self.arbiter.stop();
    }
}

fn log_error(error: &SubgraphExecutorError) {
    tracing::error!(
        error = error as &dyn std::error::Error,
        "Subgraph executor error"
    );
}

struct OwnedExecutionRequest {
    query: String,
    operation_name: Option<String>,
    variables: HashMap<String, sonic_rs::Value>,
    headers: http::HeaderMap,
    extensions: Option<HashMap<String, sonic_rs::Value>>,
}

impl OwnedExecutionRequest {
    fn from_request(request: &SubgraphExecutionRequest<'_>) -> Self {
        Self {
            query: request.query.to_string(),
            operation_name: request.operation_name.map(|s| s.to_string()),
            variables: request
                .variables
                .as_ref()
                .map(|vars| {
                    vars.iter()
                        .map(|(k, v)| (k.to_string(), (*v).clone()))
                        .collect()
                })
                .unwrap_or_default(),
            headers: request.headers.clone(),
            extensions: request.extensions.clone(),
        }
    }

    fn to_subgraph_request(&self) -> SubgraphExecutionRequest<'_> {
        SubgraphExecutionRequest {
            query: &self.query,
            dedupe: false,
            operation_name: self.operation_name.as_deref(),
            variables: Some(
                self.variables
                    .iter()
                    .map(|(k, v)| (k.as_str(), v))
                    .collect(),
            ),
            headers: self.headers.clone(),
            representations: None,
            extensions: self.extensions.clone(),
        }
    }

    fn init_payload(&self) -> Option<ConnectionInitPayload> {
        if self.headers.is_empty() {
            return None;
        }

        let fields: HashMap<String, sonic_rs::Value> = self
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.to_string(), sonic_rs::Value::from(v)))
            })
            .collect();

        if fields.is_empty() {
            None
        } else {
            Some(ConnectionInitPayload::new(fields))
        }
    }
}

#[async_trait]
impl SubgraphExecutor for WsSubgraphExecutor {
    #[tracing::instrument(level = "trace", skip_all, fields(subgraph_name = %self.subgraph_name))]
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
    ) -> SubgraphResponse<'a> {
        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        let owned_request = OwnedExecutionRequest::from_request(&execution_request);

        debug!(
            "establishing WebSocket connection to subgraph {} at {}",
            self.subgraph_name, self.endpoint
        );

        let result = self
            .arbiter
            .spawn_with(async move || {
                let init_payload = owned_request.init_payload();

                let connection = match connect(&endpoint).await {
                    Ok(conn) => conn,
                    Err(e) => {
                        let error = SubgraphExecutorError::RequestFailure(endpoint, e.to_string());
                        log_error(&error);
                        return error.to_subgraph_response(&subgraph_name);
                    }
                };

                let mut client = match WsClient::init(connection, init_payload).await {
                    Ok(client) => client,
                    Err(e) => {
                        let error = SubgraphExecutorError::RequestFailure(endpoint, e.to_string());
                        log_error(&error);
                        return error.to_subgraph_response(&subgraph_name);
                    }
                };

                debug!(
                    "WebSocket connection to subgraph {} at {} established",
                    subgraph_name, endpoint
                );

                let subgraph_request = owned_request.to_subgraph_request();
                let mut stream = client.subscribe(subgraph_request).await;

                match stream.next().await {
                    Some(response) => response,
                    None => {
                        let error = SubgraphExecutorError::RequestFailure(
                            endpoint,
                            "Stream closed without response".to_string(),
                        );
                        log_error(&error);
                        error.to_subgraph_response(&subgraph_name)
                    }
                }
            })
            .await;

        match result {
            Ok(response) => response,
            Err(_) => {
                let error = SubgraphExecutorError::RequestFailure(
                    self.endpoint.clone(),
                    "WebSocket executor channel closed".to_string(),
                );
                log_error(&error);
                error.to_subgraph_response(&self.subgraph_name)
            }
        }
    }

    #[tracing::instrument(level = "trace", skip_all, fields(subgraph_name = %self.subgraph_name))]
    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
    ) -> BoxStream<'static, SubgraphResponse<'static>> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SubgraphResponse<'static>>();

        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        let owned_request = OwnedExecutionRequest::from_request(&execution_request);

        debug!(
            "establishing WebSocket subscription connection to subgraph {} at {}",
            self.subgraph_name, self.endpoint
        );

        // no await intentionally. the arbiter runs the subscription in the background
        // and sends responses through the channel. The returned future would only resolve
        // when the subscription completes, but we want to return the stream immediately
        drop(self.arbiter.spawn_with(move || async move {
            let init_payload = owned_request.init_payload();

            let connection = match connect(&endpoint).await {
                Ok(conn) => conn,
                Err(e) => {
                    let error = SubgraphExecutorError::RequestFailure(endpoint, e.to_string());
                    log_error(&error);
                    let _ = tx.send(error.to_subgraph_response(&subgraph_name));
                    return;
                }
            };

            let mut client = match WsClient::init(connection, init_payload).await {
                Ok(client) => client,
                Err(e) => {
                    let error = SubgraphExecutorError::RequestFailure(endpoint, e.to_string());
                    log_error(&error);
                    let _ = tx.send(error.to_subgraph_response(&subgraph_name));
                    return;
                }
            };

            debug!(
                "WebSocket subscription connection to subgraph {} at {} established",
                subgraph_name, endpoint
            );

            let subgraph_request = owned_request.to_subgraph_request();
            let mut stream = client.subscribe(subgraph_request).await;

            while let Some(response) = stream.next().await {
                if tx.send(response).is_err() {
                    break;
                }
            }
        }));

        Box::pin(async_stream::stream! {
            while let Some(response) = rx.recv().await {
                yield response;
            }
        })
    }
}
