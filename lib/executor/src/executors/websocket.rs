use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::stream::BoxStream;
use futures_util::StreamExt;
use ntex::rt;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use crate::executors::error::SubgraphExecutorError;
use crate::executors::graphql_transport_ws::build_subscribe_payload;
use crate::executors::websocket_client::{connect, WsClient};
use crate::response::subgraph_response::SubgraphResponse;

pub struct WsSubgraphExecutor {
    subgraph_name: String,
    endpoint: http::Uri,
    tls_config: Option<Arc<rustls::ClientConfig>>,
}

impl WsSubgraphExecutor {
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        tls_config: Option<Arc<rustls::ClientConfig>>,
    ) -> Self {
        Self {
            subgraph_name,
            endpoint,
            tls_config,
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
    ) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        let tls_config = self.tls_config.clone();
        debug!(
            subgraph_name = subgraph_name,
            endpoint = endpoint.to_string(),
            "establishing WebSocket connection to subgraph",
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
                    subgraph_name = subgraph_name,
                    endpoint = endpoint.to_string(),
                    "WebSocket connection to subgraph established",
                );

                let mut stream = client.subscribe(subscribe_payload).await;

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
        // all subscriptions emit events into the shared active subscriptions broadcaster
        // which itself handles back-pressure by dropping old events when the buffer is full,
        // so we can use a small buffer here
        // TODO: do we thererefore need to buffer at all?
        let (tx, mut rx) =
            mpsc::channel::<Result<SubgraphResponse<'static>, SubgraphExecutorError>>(16);

        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        let tls_config = self.tls_config.clone();

        let (subscribe_payload, init_payload) = build_subscribe_payload(execution_request);

        debug!(
            subgraph_name = subgraph_name,
            endpoint = endpoint.to_string(),
            "establishing WebSocket subscription connection to subgraph",
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
                subgraph_name = subgraph_name,
                endpoint = endpoint.to_string(),
                "WebSocket subscription connection to subgraph established",
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
                            subgraph_name = subgraph_name,
                            endpoint = endpoint.to_string(),
                            "Client for subgraph subscriptions is too slow",
                        );
                        break;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        debug!(
                            subgraph_name = subgraph_name,
                            endpoint = endpoint.to_string(),
                            "Client for subgraph dropped the receiver",
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
