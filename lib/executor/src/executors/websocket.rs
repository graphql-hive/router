use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures_util::StreamExt;
use ntex::rt::Arbiter;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use crate::executors::error::SubgraphExecutorError;
use crate::executors::graphql_transport_ws::build_subscribe_payload;
use crate::executors::websocket_client::{connect, WsClient};
use crate::response::subgraph_response::SubgraphResponse;

pub struct WsSubgraphExecutor {
    arbiter: Arbiter,
    subgraph_name: String,
    endpoint: http::Uri,
}

impl WsSubgraphExecutor {
    pub fn new(subgraph_name: String, endpoint: http::Uri) -> Self {
        Self {
            // each executors its own arbiter because the WsClient is not sync+send compatible,
            // so we instead spawn a dedicated arbiter (maps to one OS thread) to run all websocket
            // connection tasks for this executor
            arbiter: Arbiter::new(),
            subgraph_name,
            endpoint,
        }
    }
}

impl Drop for WsSubgraphExecutor {
    fn drop(&mut self) {
        // arbiter does not seem to stop itself on drop, so stop it manually on executor drop
        self.arbiter.stop();
    }
}

#[async_trait]
impl SubgraphExecutor for WsSubgraphExecutor {
    fn endpoint(&self) -> &http::Uri {
        &self.endpoint
    }

    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
        _plugin_req_state: Option<&'a crate::plugin_context::PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        debug!(
            "establishing WebSocket connection to subgraph {} at {}",
            subgraph_name, endpoint
        );

        let (subscribe_payload, init_payload) = build_subscribe_payload(execution_request);

        let result = self
            .arbiter
            .spawn_with(async move || {
                let connection = match connect(&endpoint).await {
                    Ok(conn) => conn,
                    Err(e) => {
                        return Err(SubgraphExecutorError::WebSocketConnectFailure(
                            endpoint.to_string(),
                            e.to_string(),
                        ));
                    }
                };

                let mut client = match WsClient::init(connection, init_payload).await {
                    Ok(client) => client,
                    Err(e) => {
                        return Err(SubgraphExecutorError::WebSocketHandshakeFailure(
                            endpoint.to_string(),
                            e.to_string(),
                        ));
                    }
                };

                debug!(
                    "WebSocket connection to subgraph {} at {} established",
                    subgraph_name, endpoint
                );

                let mut stream = client.subscribe(subscribe_payload).await;

                match stream.next().await {
                    Some(response) => Ok(response),
                    None => Err(SubgraphExecutorError::WebSocketStreamClosedEmpty(
                        endpoint.to_string(),
                    )),
                }
            })
            .await;

        result
            .map_err(|_| SubgraphExecutorError::WebSocketArbiterChannelClosed)
            .and_then(|r| r)
    }

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
    ) -> Result<
        BoxStream<'static, Result<SubgraphResponse<'static>, SubgraphExecutorError>>,
        SubgraphExecutorError,
    > {
        // all subscriptions emit events into the shared active subscriptions broadcaster
        // which itself handles back-pressure by dropping old events when the buffer is full,
        // so we can use a small buffer here
        // TODO: do we thererefore need to buffer at all?
        let (tx, mut rx) =
            mpsc::channel::<Result<SubgraphResponse<'static>, SubgraphExecutorError>>(16);

        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();

        let (subscribe_payload, init_payload) = build_subscribe_payload(execution_request);

        debug!(
            "establishing WebSocket subscription connection to subgraph {} at {}",
            self.subgraph_name, self.endpoint
        );

        // no await intentionally. the arbiter runs the subscription in the background
        // and sends responses through the channel. The returned future would only resolve
        // when the subscription completes, but we want to return the stream immediately
        drop(self.arbiter.spawn_with(move || async move {
            let connection = match connect(&endpoint).await {
                Ok(conn) => conn,
                Err(e) => {
                    let _ = tx.try_send(Err(SubgraphExecutorError::WebSocketConnectFailure(
                        endpoint.to_string(),
                        e.to_string(),
                    )));
                    return;
                }
            };

            let mut client = match WsClient::init(connection, init_payload).await {
                Ok(client) => client,
                Err(e) => {
                    let _ = tx.try_send(Err(SubgraphExecutorError::WebSocketHandshakeFailure(
                        endpoint.to_string(),
                        e.to_string(),
                    )));
                    return;
                }
            };

            debug!(
                "WebSocket subscription connection to subgraph {} at {} established",
                subgraph_name, endpoint
            );

            let mut stream = client.subscribe(subscribe_payload).await;

            while let Some(response) = stream.next().await {
                match tx.try_send(Ok(response)) {
                    Ok(()) => (),
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        // if the channel is full it means the consuming client is too slow and unable to keep
                        // up. we terminate the subscription without an error message because it anyways cant
                        // go through
                        warn!(
                            "Client for subgraph {} at {} subscriptions is too slow",
                            subgraph_name, endpoint
                        );
                        break;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        debug!(
                            "Client for subgraph {} at {} dropped the receiver",
                            subgraph_name, endpoint
                        );
                        break;
                    }
                }
            }
        }));

        Ok(Box::pin(async_stream::stream! {
            while let Some(response) = rx.recv().await {
                yield response;
            }
        }))
    }
}
