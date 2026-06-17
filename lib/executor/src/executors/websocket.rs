use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::stream::BoxStream;
use futures_util::StreamExt;
use hive_router_internal::telemetry::TelemetryContext;
use ntex::rt;
use tokio::sync::mpsc;
use tracing::debug;

use crate::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use crate::executors::error::SubgraphExecutorError;
use crate::executors::graphql_transport_ws::build_subscribe_payload;
use crate::executors::subscription_buffer::{forward_or_drop, SubscriptionItem};
use crate::executors::websocket_client::{connect, WsClient};
use crate::response::subgraph_response::SubgraphResponse;

pub struct WsSubgraphExecutor {
    subgraph_name: String,
    endpoint: http::Uri,
    tls_config: Option<Arc<rustls::ClientConfig>>,
    telemetry_context: Arc<TelemetryContext>,
    buffer_capacity: usize,
}

impl WsSubgraphExecutor {
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        tls_config: Option<Arc<rustls::ClientConfig>>,
        telemetry_context: Arc<TelemetryContext>,
        buffer_capacity: usize,
    ) -> Self {
        Self {
            subgraph_name,
            endpoint,
            tls_config,
            telemetry_context,
            buffer_capacity,
        }
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
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        let tls_config = self.tls_config.clone();
        let custom_scalar_paths = execution_request.custom_scalar_paths.cloned();
        debug!(
            "establishing WebSocket connection to subgraph {} at {}",
            subgraph_name, endpoint
        );

        let (subscribe_payload, init_payload) = build_subscribe_payload(execution_request);

        let (tx, rx) = oneshot::channel();

        // run this on ntex runtime instead of Handle::spawn because the websocket path builds
        // and awaits futures that capture ntex local types like Rc and RefCell via WsClient.
        // those futures are not Send, so they cannot cross a tokio multi-threaded spawn boundary.
        // ntex::rt::spawn keeps the whole websocket flow on the local ntex runtime, while this
        // async_trait method still stays Send by awaiting only the futures oneshot receiver here.
        // this task ends after the first websocket response is forwarded through the oneshot,
        // or earlier if connect/init fails.
        rt::spawn(async move {
            let result = async {
                let connection = match connect(&endpoint, tls_config).await {
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

                let mut stream = client
                    .subscribe(subscribe_payload, custom_scalar_paths)
                    .await;

                match stream.next().await {
                    Some(response) => Ok(response),
                    None => Err(SubgraphExecutorError::WebSocketStreamClosedEmpty(
                        endpoint.to_string(),
                    )),
                }
            }
            .await;

            let _ = tx.send(result);
        });

        rx.await
            .map_err(|_| SubgraphExecutorError::WebSocketArbiterChannelClosed)?
    }

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
    ) -> Result<
        BoxStream<'static, Result<SubgraphResponse<'static>, SubgraphExecutorError>>,
        SubgraphExecutorError,
    > {
        // The subgraph reader task must drain the WebSocket eagerly (graphql-ws ping/pong liveness
        // and connection multiplexing forbid back-pressuring the socket), so it forwards events
        // into a bounded buffer and sheds load there instead: when the consumer - per-event plan
        // execution - falls behind and the buffer is full, the newest event is dropped
        // (drop-latest) and the subscription is kept alive. HTTP (SSE/multipart) subscriptions
        // apply the identical policy via `subscription_buffer::buffered_drop_latest`.
        let (tx, mut rx) = mpsc::channel::<SubscriptionItem>(self.buffer_capacity.max(1));

        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        let tls_config = self.tls_config.clone();
        let telemetry_context = self.telemetry_context.clone();
        let custom_scalar_paths = execution_request.custom_scalar_paths.cloned();

        let (subscribe_payload, init_payload) = build_subscribe_payload(execution_request);

        debug!(
            "establishing WebSocket subscription connection to subgraph {} at {}",
            self.subgraph_name, self.endpoint
        );

        // no await intentionally. the task runs the subscription in the background and forwards
        // responses through the bounded buffer. The spawned future stays local to the ntex runtime,
        // so it can hold non-Send websocket client state. this task ends when the websocket stream
        // completes, the consumer drops the receiver, or the connection fails.
        drop(rt::spawn(async move {
            let connection = match connect(&endpoint, tls_config).await {
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

            let mut stream = client
                .subscribe(subscribe_payload, custom_scalar_paths)
                .await;

            while let Some(response) = stream.next().await {
                if !forward_or_drop(&tx, Ok(response), &subgraph_name, &telemetry_context) {
                    break;
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
