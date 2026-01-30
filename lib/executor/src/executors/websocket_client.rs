use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

use bytes::Bytes;
use futures::{stream::LocalBoxStream, StreamExt};
use ntex::{
    channel::mpsc,
    io::Sealed,
    rt,
    ws::{
        self,
        error::{WsClientBuilderError, WsClientError, WsError},
        WsClient, WsConnection, WsSink,
    },
};
use tracing::{error, trace, warn};

use crate::{
    executors::graphql_transport_ws::{ClientMessage, CloseCode, ExecutionRequest, ServerMessage},
    response::{graphql_error::GraphQLError, subgraph_response::SubgraphResponse},
};

#[derive(Debug, thiserror::Error)]
pub enum WsConnectError {
    #[error("WebSocket client error: {0}")]
    Client(#[from] WsClientError),
    #[error("WebSocket client builder error: {0}")]
    Builder(#[from] WsClientBuilderError),
}

pub async fn connect(url: &str) -> Result<WsConnection<Sealed>, WsConnectError> {
    if url.starts_with("wss://") {
        use tls_openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

        let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
        builder.set_verify(SslVerifyMode::PEER);
        let _ = builder
            .set_alpn_protos(b"\x08http/1.1")
            .map_err(|e| error!("Cannot set alpn protocol: {e:?}"));

        let ws_client = WsClient::build(url)
            .timeout(ntex::time::Seconds(60))
            .openssl(builder.build())
            .take()
            .finish()?;

        Ok(ws_client.connect().await?.seal())
    } else {
        let ws_client = WsClient::build(url)
            .timeout(ntex::time::Seconds(60))
            .finish()?;

        Ok(ws_client.connect().await?.seal())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum GraphQLTransportWsClientError {
    #[error("Connection closed: {code} - {reason}")]
    ConnectionClosed { code: u16, reason: String },
    #[error("Send error: {0}")]
    SendError(String),
    #[error("Receive error: {0}")]
    ReceiveError(String),
    #[error("Invalid message: {0}")]
    InvalidMessage(String),
    #[error("Connection acknowledgement timeout")]
    ConnectionAcknowledgementTimeout,
}

fn error_response(message: String) -> SubgraphResponse<'static> {
    SubgraphResponse {
        errors: Some(vec![GraphQLError::from_message_and_code(
            message,
            "WEBSOCKET_ERROR",
        )]),
        ..Default::default()
    }
}

type SubscriptionSender = mpsc::Sender<SubgraphResponse<'static>>;

type SubscriptionsMap = Rc<RefCell<HashMap<String, SubscriptionSender>>>;

/// Ensures a subscription is cleaned up when dropped.
struct SubscriptionGuard {
    subscriptions: SubscriptionsMap,
    sink: WsSink,
    id: String,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        // only send complete message if the subscription is still active - client cancelled.
        // if the server sent the complete/error message, the subscription would've been removed
        // by the dispatcher so no complete message would be sent from the client back to the server
        if self.subscriptions.borrow_mut().remove(&self.id).is_some() {
            let id = self.id.clone();
            let sink = self.sink.clone();

            // sending is async, so spawn a task to do it
            rt::spawn(async move {
                let complete_msg = ClientMessage::Complete { id: id.clone() };
                if let Ok(msg_str) = sonic_rs::to_string(&complete_msg) {
                    let _ = sink.send(ws::Message::Text(msg_str.into())).await;
                    trace!(id = %id, "Sent complete message on stream drop");
                }
            });
        }
    }
}

/// GraphQL over WebSocket client implementing the graphql-transport-ws protocol.
///
/// This client is designed for single-threaded use with ntex's runtime.
/// It is not Send/Sync due to ntex's Rc-based internal types.
///
/// Supports multiplexing multiple subscriptions over a single WebSocket connection,
/// it does so by spawning a background task to handle incoming messages and dispatch them
/// to the appropriate subscription streams as well as handling connection-level messages.
pub struct GraphQLTransportWSClient {
    sink: WsSink,
    subscriptions: SubscriptionsMap,
    next_id: u64,
}

impl GraphQLTransportWSClient {
    /// Initialize a new GraphQL over WebSocket client.
    ///
    /// This performs the connection handshake and blocks until the server acknowledges
    /// the connection. Returns an error if the handshake fails or times out.
    pub async fn init(
        connection: WsConnection<Sealed>,
        payload: Option<HashMap<String, sonic_rs::Value>>,
        timeout: Option<Duration>,
    ) -> Result<Self, GraphQLTransportWsClientError> {
        let sink = connection.sink();
        let mut receiver = connection.receiver();

        let init_msg = ClientMessage::ConnectionInit {
            payload: payload.map(|fields| {
                crate::executors::graphql_transport_ws::ConnectionInitPayload { fields }
            }),
        };

        let msg_str = sonic_rs::to_string(&init_msg)
            .map_err(|e| GraphQLTransportWsClientError::SendError(e.to_string()))?;

        sink.send(ws::Message::Text(msg_str.into()))
            .await
            .map_err(|e| GraphQLTransportWsClientError::SendError(e.to_string()))?;

        let timeout_duration = timeout.unwrap_or(Duration::from_secs(10));

        tokio::select! {
            maybe_frame = receiver.next() => {
                match maybe_frame {
                    Some(Ok(frame)) => match frame {
                        ws::Frame::Text(text) => {
                            let text_str = String::from_utf8(text.to_vec()).map_err(|e| {
                                GraphQLTransportWsClientError::InvalidMessage(e.to_string())
                            })?;
                            let server_msg: ServerMessage =
                                sonic_rs::from_str(&text_str).map_err(|e| {
                                    GraphQLTransportWsClientError::InvalidMessage(e.to_string())
                                })?;

                            match server_msg {
                                ServerMessage::ConnectionAck { .. } => Ok(()),
                                _ => Err(GraphQLTransportWsClientError::InvalidMessage(
                                    "Expected ConnectionAck".to_string(),
                                )),
                            }
                        }
                        ws::Frame::Close(reason) => {
                            let (code, desc) = reason
                                .map(|r| (r.code.into(), r.description.unwrap_or_default()))
                                .unwrap_or((1000, String::new()));
                            Err(GraphQLTransportWsClientError::ConnectionClosed { code, reason: desc })
                        }
                        _ => Err(GraphQLTransportWsClientError::InvalidMessage(
                            "Unexpected frame type".to_string(),
                        )),
                    },
                    Some(Err(e)) => Err(GraphQLTransportWsClientError::ReceiveError(format!(
                        "{:?}",
                        e
                    ))),
                    None => Err(GraphQLTransportWsClientError::ConnectionClosed {
                        code: 1000,
                        reason: "Connection closed".to_string(),
                    }),
                }
            }
            _ = ntex::time::sleep(timeout_duration) => {
                let _ = sink.send(CloseCode::ConnectionAcknowledgementTimeout.into())
                    .await;
                Err(GraphQLTransportWsClientError::ConnectionAcknowledgementTimeout)
            }
        }?;

        let subscriptions: SubscriptionsMap = Rc::new(RefCell::new(HashMap::new()));

        // Spawn background task to dispatch messages to subscriptions
        rt::spawn(dispatch_messages(
            receiver,
            sink.clone(),
            subscriptions.clone(),
        ));

        Ok(Self {
            sink,
            subscriptions,
            next_id: 1,
        })
    }

    fn next_subscription_id(&mut self) -> String {
        let id = self.next_id;
        self.next_id += 1;
        id.to_string()
    }

    /// Execute a GraphQL operation (query, mutation, or subscription) over WebSocket.
    ///
    /// Returns a stream of responses. The stream completes when the server sends
    /// a Complete message, or can be cancelled by dropping the stream.
    ///
    /// Multiple subscriptions can be active simultaneously on the same connection.
    pub async fn subscribe(
        &mut self,
        query: &str,
        operation_name: Option<&str>,
        variables: Option<HashMap<String, sonic_rs::Value>>,
        extensions: Option<HashMap<String, sonic_rs::Value>>,
    ) -> Result<LocalBoxStream<'static, SubgraphResponse<'static>>, GraphQLTransportWsClientError>
    {
        let subscribe_id = self.next_subscription_id();

        let subscribe_msg = ClientMessage::Subscribe {
            id: subscribe_id.clone(),
            payload: ExecutionRequest {
                query: query.to_string(),
                operation_name: operation_name.map(|s| s.to_string()),
                variables: variables.unwrap_or_default(),
                extensions,
            },
        };

        let msg_str = sonic_rs::to_string(&subscribe_msg)
            .map_err(|e| GraphQLTransportWsClientError::SendError(e.to_string()))?;

        self.sink
            .send(ws::Message::Text(msg_str.into()))
            .await
            .map_err(|e| GraphQLTransportWsClientError::SendError(e.to_string()))?;

        trace!(id = %subscribe_id, "Subscribe message sent");

        let (tx, rx) = mpsc::channel();

        self.subscriptions
            .borrow_mut()
            .insert(subscribe_id.clone(), tx);

        let subscriptions = self.subscriptions.clone();
        let sink = self.sink.clone();

        Ok(Box::pin(async_stream::stream! {
            let mut rx = rx;
            let _guard = SubscriptionGuard {
                subscriptions,
                sink,
                id: subscribe_id,
            };

            loop {
                match rx.next().await {
                    Some(response) => {
                        // the response specific to THIS subscription (matching by id)
                        yield response;
                    }
                    None => {
                        // channel closed
                        break;
                    }
                }
            }
        }))
    }
}

impl Drop for GraphQLTransportWSClient {
    fn drop(&mut self) {
        let sink = self.sink.clone();
        // sending is async, so spawn a task to do it
        rt::spawn(async move {
            let _ = sink
                .send(ws::Message::Close(Some(ws::CloseCode::Normal.into())))
                .await;
        });
    }
}

struct MessageDispatcherGuard {
    subscriptions: SubscriptionsMap,
}

impl Drop for MessageDispatcherGuard {
    fn drop(&mut self) {
        let err_msg = "Message dispatcher closed";
        for (_, tx) in self.subscriptions.borrow_mut().drain() {
            let _ = tx.send(error_response(err_msg.to_string()));
            tx.close();
        }
    }
}

async fn dispatch_messages(
    mut receiver: mpsc::Receiver<Result<ws::Frame, WsError<()>>>,
    sink: WsSink,
    subscriptions: SubscriptionsMap,
) {
    let _guard = MessageDispatcherGuard {
        subscriptions: subscriptions.clone(),
    };

    loop {
        match receiver.next().await {
            Some(Ok(ws::Frame::Text(text))) => {
                let text = match String::from_utf8(text.to_vec()) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Invalid UTF-8 in message: {}", e);
                        let _ = sink
                            .send(ws::Message::Close(Some(
                                // this one is not in the CloseCode enum because it's an internal WebSocket
                                // transport error that has nothing to do with GraphQL over WebSockets
                                ws::CloseReason {
                                    code: ws::CloseCode::Unsupported,
                                    description: Some("Invalid UTF-8 in message".into()),
                                },
                            )))
                            .await;
                        return;
                    }
                };

                let server_msg: ServerMessage = match sonic_rs::from_str(&text) {
                    Ok(msg) => msg,
                    Err(e) => {
                        error!("Failed to parse server message to JSON: {}", e);
                        let _ = sink
                            .send(
                                CloseCode::BadResponse("Invalid message received from server")
                                    .into(),
                            )
                            .await;
                        return;
                    }
                };

                match server_msg {
                    ServerMessage::Next { id, payload } => {
                        trace!(id = %id, "Received next message");
                        if let Some(tx) = subscriptions.borrow().get(&id) {
                            let payload_bytes =
                                Bytes::from(sonic_rs::to_string(&payload).unwrap_or_default());
                            let response =
                                match SubgraphResponse::deserialize_from_bytes(payload_bytes) {
                                    Ok(response) => response,
                                    Err(e) => {
                                        warn!("Failed to deserialize payload: {}", e);
                                        error_response(format!(
                                            "Failed to deserialize payload: {}",
                                            e
                                        ))
                                    }
                                };
                            let _ = tx.send(response);
                        }
                    }
                    ServerMessage::Error { id, payload } => {
                        trace!(id = %id, "Received error message");
                        if let Some(tx) = subscriptions.borrow_mut().remove(&id) {
                            let _ = tx.send(SubgraphResponse {
                                errors: Some(payload),
                                ..Default::default()
                            });
                            tx.close();
                        }
                    }
                    ServerMessage::Complete { id } => {
                        trace!(id = %id, "Received complete message");
                        if let Some(tx) = subscriptions.borrow_mut().remove(&id) {
                            tx.close();
                        }
                    }
                    // TODO: handle ping
                    ServerMessage::Pong {} => {
                        trace!("Received pong");
                    }
                    ServerMessage::ConnectionAck { .. } => {
                        trace!("Received unexpected ConnectionAck");
                    }
                }
            }
            Some(Ok(ws::Frame::Ping(data))) => {
                let _ = sink.send(ws::Message::Pong(data)).await;
            }
            Some(Ok(ws::Frame::Close(_reason))) => {
                // TODO: what to do?
                return;
            }
            Some(Err(e)) => {
                error!("WebSocket error: {:?}", e);
                return;
            }
            None => {
                return;
            }
            _ => {}
        }
    }
}
