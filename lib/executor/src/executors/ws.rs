use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures_util::StreamExt;
use ntex::rt::Arbiter;
use tokio::sync::Semaphore;
use tracing::debug;

use crate::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use crate::executors::error::SubgraphExecutorError;
use crate::executors::websocket_client::{connect, WsClient};
use crate::response::subgraph_response::SubgraphResponse;

pub struct WsSubgraphExecutor {
    arbiter: Arbiter,
    subgraph_name: String,
    endpoint: http::Uri,
    semaphore: Arc<Semaphore>,
}

impl WsSubgraphExecutor {
    pub fn new(subgraph_name: String, endpoint: http::Uri, semaphore: Arc<Semaphore>) -> Self {
        Self {
            // each executors its own arbiter because the WsClient is not sync+send compatible,
            // so we instead spawn a dedicated arbiter (maps to one OS thread) to run all websocket
            // connection tasks for this executor
            arbiter: Arbiter::new(),
            subgraph_name,
            endpoint,
            semaphore,
        }
    }
}

impl Drop for WsSubgraphExecutor {
    fn drop(&mut self) {
        // arbiter does not seem to stop itself on drop, so stop it manually on executor drop
        self.arbiter.stop();
    }
}

fn log_error(error: &SubgraphExecutorError) {
    tracing::error!(
        error = error as &dyn std::error::Error,
        "Subgraph executor error"
    );
}

#[async_trait]
impl SubgraphExecutor for WsSubgraphExecutor {
    #[tracing::instrument(level = "trace", skip_all, fields(subgraph_name = %self.subgraph_name))]
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
    ) -> SubgraphResponse<'a> {
        // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
        // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
        let _permit = self.semaphore.acquire().await.unwrap();

        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();

        let query = execution_request.query.to_string();
        let operation_name = execution_request.operation_name.map(|s| s.to_string());
        let variables: HashMap<String, sonic_rs::Value> = execution_request
            .variables
            .as_ref()
            .map(|vars| {
                vars.iter()
                    .map(|(k, v)| (k.to_string(), (*v).clone()))
                    .collect()
            })
            .unwrap_or_default();

        // TODO: should we add request.headers to extensions.headers of the execution request?
        //       I dont think that subgraphs out there would expect headers to be sent anywhere else
        //       aside from the connection init message though
        let headers = execution_request.headers.clone();
        let extensions = execution_request.extensions.clone();

        debug!(
            "establishing WebSocket connection to subgraph {} at {}",
            self.subgraph_name, self.endpoint
        );

        let result = self
            .arbiter
            .spawn_with(async move || {
                let init_payload = if headers.is_empty() {
                    None
                } else {
                    Some(headers.into())
                };

                let connection = match connect(&endpoint).await {
                    Ok(conn) => conn,
                    Err(e) => {
                        let error = SubgraphExecutorError::RequestFailure(
                            endpoint.to_string(),
                            e.to_string(),
                        );
                        log_error(&error);
                        return error.to_subgraph_response(&subgraph_name);
                    }
                };

                let mut client = match WsClient::init(connection, init_payload).await {
                    Ok(client) => client,
                    Err(e) => {
                        let error = SubgraphExecutorError::RequestFailure(
                            endpoint.to_string(),
                            e.to_string(),
                        );
                        log_error(&error);
                        return error.to_subgraph_response(&subgraph_name);
                    }
                };

                debug!(
                    "WebSocket connection to subgraph {} at {} established",
                    subgraph_name, endpoint
                );

                let mut stream = client
                    .subscribe(query, operation_name, variables, extensions)
                    .await;

                match stream.next().await {
                    Some(response) => response,
                    None => {
                        let error = SubgraphExecutorError::RequestFailure(
                            endpoint.to_string(),
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
                    self.endpoint.to_string(),
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
        // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
        // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
        let _permit = self.semaphore.acquire().await.unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SubgraphResponse<'static>>();

        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();

        let query = execution_request.query.to_string();
        let operation_name = execution_request.operation_name.map(|s| s.to_string());
        let variables: HashMap<String, sonic_rs::Value> = execution_request
            .variables
            .as_ref()
            .map(|vars| {
                vars.iter()
                    .map(|(k, v)| (k.to_string(), (*v).clone()))
                    .collect()
            })
            .unwrap_or_default();

        // TODO: should we add request.headers to extensions.headers of the execution request?
        //       I dont think that subgraphs out there would expect headers to be sent anywhere else
        //       aside from the connection init message though
        let headers = execution_request.headers.clone();
        let extensions = execution_request.extensions.clone();

        debug!(
            "establishing WebSocket subscription connection to subgraph {} at {}",
            self.subgraph_name, self.endpoint
        );

        // no await intentionally. the arbiter runs the subscription in the background
        // and sends responses through the channel. The returned future would only resolve
        // when the subscription completes, but we want to return the stream immediately
        drop(self.arbiter.spawn_with(move || async move {
            let init_payload = if headers.is_empty() {
                None
            } else {
                Some(headers.into())
            };

            let connection = match connect(&endpoint).await {
                Ok(conn) => conn,
                Err(e) => {
                    let error =
                        SubgraphExecutorError::RequestFailure(endpoint.to_string(), e.to_string());
                    log_error(&error);
                    let _ = tx.send(error.to_subgraph_response(&subgraph_name));
                    return;
                }
            };

            let mut client = match WsClient::init(connection, init_payload).await {
                Ok(client) => client,
                Err(e) => {
                    let error =
                        SubgraphExecutorError::RequestFailure(endpoint.to_string(), e.to_string());
                    log_error(&error);
                    let _ = tx.send(error.to_subgraph_response(&subgraph_name));
                    return;
                }
            };

            debug!(
                "WebSocket subscription connection to subgraph {} at {} established",
                subgraph_name, endpoint
            );

            let mut stream = client
                .subscribe(query, operation_name, variables, extensions)
                .await;

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
