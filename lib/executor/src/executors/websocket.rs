use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::stream::BoxStream;
use futures_util::StreamExt;
use ntex::rt;
use tokio::sync::mpsc;
use tracing::debug;

use crate::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use crate::executors::error::SubgraphExecutorError;
use crate::executors::graphql_transport_ws::build_subscribe_payload;
use crate::executors::subscription_buffer::{drain_into, receiver_stream};
use crate::executors::websocket_client::{connect, WsClient};
use crate::response::subgraph_response::SubgraphResponse;

pub struct WsSubgraphExecutor {
    subgraph_name: String,
    endpoint: http::Uri,
    tls_config: Option<Arc<rustls::ClientConfig>>,
    buffer_capacity: usize,
}

impl WsSubgraphExecutor {
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        tls_config: Option<Arc<rustls::ClientConfig>>,
        buffer_capacity: usize,
    ) -> Self {
        Self {
            subgraph_name,
            endpoint,
            tls_config,
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
        // buffer decouples the emitting subgraph from slow downstream consumers, dropping
        // messages under backpressure instead of throttling the subgraph
        let (tx, rx) = mpsc::channel::<Result<SubgraphResponse<'static>, SubgraphExecutorError>>(
            self.buffer_capacity,
        );

        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        let tls_config = self.tls_config.clone();
        let custom_scalar_paths = execution_request.custom_scalar_paths.cloned();

        let (subscribe_payload, init_payload) = build_subscribe_payload(execution_request);

        debug!(
            "establishing WebSocket subscription connection to subgraph {} at {}",
            self.subgraph_name, self.endpoint
        );

        // no await intentionally. the task runs the subscription in the background
        // and sends responses through the channel. The spawned future itself stays local
        // to ntex runtime, so it can hold non-Send websocket client state.
        // this task ends when the websocket stream completes, the client drops the receiver,
        // or back-pressure fills the channel and we terminate the subscription.
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

            let stream = client
                .subscribe(subscribe_payload, custom_scalar_paths)
                .await
                .map(Ok);

            drain_into(stream, tx, &subgraph_name, &endpoint.to_string()).await;
        }));

        Ok(receiver_stream(rx))
    }
}
