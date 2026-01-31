use bytes::Bytes;
use std::{cell::RefCell, rc::Rc};

use futures::{stream::LocalBoxStream, StreamExt};
use ntex::{
    channel::{mpsc, oneshot},
    io::Sealed,
    rt,
    ws::{self, error::WsError, WsClient as NtexWsClient, WsConnection, WsSink},
};
use tracing::{debug, error, trace};

use crate::{
    executors::graphql_transport_ws::WS_SUBPROTOCOL, response::subgraph_response::SubgraphResponse,
};
use crate::{
    executors::{
        graphql_transport_ws::{
            ClientMessage, CloseCode, ConnectionInitPayload, ExecutionRequest, ServerMessage,
        },
        websocket_common::{
            handshake_timeout, heartbeat, parse_frame_to_text, FrameNotParsedToText, WsState,
        },
    },
    response::graphql_error::GraphQLError,
};

#[derive(Debug, thiserror::Error)]
pub enum WsConnectError {
    #[error("WebSocket client error: {0}")]
    Client(#[from] ws::error::WsClientError),
    #[error("WebSocket client builder error: {0}")]
    Builder(#[from] ws::error::WsClientBuilderError),
}

#[derive(Debug, thiserror::Error)]
pub enum WsInitError {
    #[error("Connection acknowledgement receiver failed")]
    ConnectionAckReceiverError,
    #[error("Connection acknowledgement receiver closed")]
    ConnectionAckReceiverClosed,
    #[error("Connection closed before acknowledgement")]
    ConnectionClosedBeforeAck,
    #[error("Invalid message received during acknowledgement")]
    InvalidMessage,
    #[error("Wrong message received before connection acknowledgement")]
    WrongMessageBeforeAck,
}

pub async fn connect(url: &str) -> Result<WsConnection<ntex::io::Sealed>, WsConnectError> {
    if url.starts_with("wss://") {
        use tls_openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

        let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
        builder.set_verify(SslVerifyMode::PEER);
        let _ = builder
            .set_alpn_protos(b"\x08http/1.1")
            .map_err(|e| tracing::error!("Cannot set alpn protocol: {e:?}"));

        let ws_client = NtexWsClient::build(url)
            .protocols([WS_SUBPROTOCOL])
            .timeout(ntex::time::Seconds(60))
            .openssl(builder.build())
            .take()
            .finish()?;

        Ok(ws_client.connect().await?.seal())
    } else {
        let ws_client = NtexWsClient::build(url)
            .protocols([WS_SUBPROTOCOL])
            .timeout(ntex::time::Seconds(60))
            .finish()?;

        Ok(ws_client.connect().await?.seal())
    }
}

/// The client's WebSocket state. Its subscriptions map subscription IDs to their response senders.
type WsStateRef = Rc<RefCell<WsState<mpsc::Sender<SubgraphResponse<'static>>>>>;

/// GraphQL over WebSocket client implementing the graphql-transport-ws protocol.
///
/// This client is designed for single-threaded use with ntex's runtime.
/// It is not Send/Sync due to ntex's Rc-based internal types.
///
/// Supports multiplexing multiple subscriptions over a single WebSocket connection,
/// it does so by spawning a background task to handle incoming messages and dispatch them
/// to the appropriate subscription streams as well as handling connection-level messages.
pub struct WsClient {
    sink: ws::WsSink,
    state: WsStateRef,
    next_subscription_id: u64,
    _heartbeat_stop_tx: Option<oneshot::Sender<()>>,
}

impl WsClient {
    /// Initialize a new GraphQL over WebSocket client.
    ///
    /// This sends the connection init message and waits for the server to acknowledge.
    /// After acknowledgement, spawns background tasks for:
    /// - Message dispatching
    /// - Heartbeat pings
    ///
    /// Returns an error if the connection is closed before acknowledgement.
    pub async fn init(
        connection: WsConnection<Sealed>,
        payload: Option<ConnectionInitPayload>,
    ) -> Result<Self, WsInitError> {
        debug!("Initialising WebSocket client connection");

        let sink = connection.sink();
        let mut receiver = connection.receiver();

        let (acknowledged_tx, acknowledged_rx) = oneshot::channel();

        let state: WsStateRef = Rc::new(RefCell::new(WsState::new(acknowledged_tx)));

        // heartbeats
        let (heartbeat_stop_tx, heartbeat_stop_rx) = oneshot::channel();
        rt::spawn(heartbeat(state.clone(), sink.clone(), heartbeat_stop_rx));

        // handshake timeout monitor will close connection if no ack received in time
        rt::spawn(handshake_timeout(
            state.clone(),
            sink.clone(),
            acknowledged_rx,
            CloseCode::ConnectionAcknowledgementTimeout,
        ));

        // send init and wait for ack or connection close
        let _ = sink.send(ClientMessage::init(payload)).await;
        loop {
            match receiver.next().await {
                Some(Ok(frame)) => {
                    match parse_frame_to_text(frame, &state) {
                        Ok(text) => {
                            let server_msg = match text_to_server_message(&text) {
                                Ok(msg) => msg,
                                Err(msg) => {
                                    let _ = sink.send(msg).await;
                                    return Err(WsInitError::InvalidMessage);
                                }
                            };

                            match server_msg {
                                ServerMessage::ConnectionAck {} => {
                                    state.borrow_mut().handshake_received = true;
                                    state.borrow_mut().complete_handshake();
                                    debug!("Connection acknowledged");
                                    break;
                                }
                                ServerMessage::Ping {} => {
                                    let _ = sink.send(ClientMessage::pong()).await;
                                }
                                ServerMessage::Pong {} => {}
                                _ => {
                                    // any other message before ack is an error
                                    error!(
                                        "Wrong message received before ConnectionAck: {:?}",
                                        server_msg
                                    );
                                    let _ = sink.send(CloseCode::Unauthorized.into()).await;
                                    return Err(WsInitError::WrongMessageBeforeAck);
                                }
                            }
                        }
                        Err(FrameNotParsedToText::Message(msg)) => {
                            // this is safe to send indenependently of ack, it could be a ping/pong
                            // or a close frame due to parsing issues
                            let _ = sink.send(msg).await;
                        }
                        Err(FrameNotParsedToText::Closed) => {
                            debug!("Connection closed before acknowledgement");
                            return Err(WsInitError::ConnectionClosedBeforeAck);
                        }
                        Err(FrameNotParsedToText::None) => {}
                    }
                }
                Some(Err(e)) => {
                    error!("WebSocket receiver error during init: {:?}", e);
                    return Err(WsInitError::ConnectionAckReceiverError);
                }
                None => {
                    debug!("WebSocket receiver closed during init");
                    return Err(WsInitError::ConnectionAckReceiverClosed);
                }
            }
        }

        let dispatcher_state = state.clone();
        let dispatcher_sink = sink.clone();
        rt::spawn(async move {
            let _guard = DispatcherGuard {
                state: dispatcher_state.clone(),
            };
            dispatch_loop(receiver, dispatcher_sink, dispatcher_state).await;
        });

        Ok(Self {
            sink,
            state,
            next_subscription_id: 1,
            _heartbeat_stop_tx: Some(heartbeat_stop_tx),
        })
    }

    fn next_subscription_id(&mut self) -> String {
        let id = self.next_subscription_id;
        self.next_subscription_id += 1;
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
        query: String,
        operation_name: Option<String>,
        variables: std::collections::HashMap<String, sonic_rs::Value>,
        extensions: Option<std::collections::HashMap<String, sonic_rs::Value>>,
    ) -> LocalBoxStream<'static, SubgraphResponse<'static>> {
        let subscribe_id = self.next_subscription_id();

        let execution_request = ExecutionRequest {
            query,
            operation_name,
            variables,
            extensions,
        };

        let _ = self
            .sink
            .send(ClientMessage::subscribe(subscribe_id.clone(), execution_request).into())
            .await;

        trace!(id = %subscribe_id, "Subscribe message sent");

        let (tx, rx) = mpsc::channel();

        self.state
            .borrow_mut()
            .subscriptions
            .insert(subscribe_id.clone(), tx);

        let state = self.state.clone();
        let sink = self.sink.clone();

        Box::pin(async_stream::stream! {
            let mut rx = rx;
            let _guard = SubscriptionGuard {
                state,
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
        })
    }
}

impl Drop for WsClient {
    fn drop(&mut self) {
        // heartbeat_stop_tx will be dropped automatically, stopping the heartbeat task

        // sending is async, so spawn a task to do it
        let sink = self.sink.clone();
        rt::spawn(async move {
            // TODO: client can be dropped but already closed by server, should be ok though
            let _ = sink
                .send(ws::Message::Close(Some(ws::CloseCode::Normal.into())))
                .await;
        });
    }
}

/// Ensures a subscription is cleaned up when dropped.
struct SubscriptionGuard {
    state: WsStateRef,
    sink: WsSink,
    id: String,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        // only send complete message if the subscription is still active - client cancelled.
        // if the server sent the complete/error message, the subscription would've been removed
        // by the dispatcher so no complete message would be sent from the client back to the server
        if self
            .state
            .borrow_mut()
            .subscriptions
            .remove(&self.id)
            .is_some()
        {
            let id = self.id.clone();
            let sink = self.sink.clone();

            // sending is async, so spawn a task to do it
            rt::spawn(async move {
                let _ = sink.send(ClientMessage::complete(id.clone()).into()).await;
            });
        }
    }
}

/// Dispatch loop handling WebSocket messages and distributing them accordingly across subscriptions.
async fn dispatch_loop(
    mut receiver: mpsc::Receiver<Result<ws::Frame, WsError<()>>>,
    sink: WsSink,
    state: WsStateRef,
) {
    loop {
        match receiver.next().await {
            Some(Ok(frame)) => {
                match parse_frame_to_text(frame, &state) {
                    Ok(text) => {
                        if let Some(msg) = handle_text_frame(text, &state) {
                            if send_and_is_closed(sink.clone(), msg).await {
                                return;
                            }
                        }
                    }
                    Err(FrameNotParsedToText::Message(msg)) => {
                        if send_and_is_closed(sink.clone(), msg).await {
                            return;
                        }
                    }
                    Err(FrameNotParsedToText::Closed) => {
                        // notify all subscriptions that the connection was closed
                        for (_, tx) in state.borrow_mut().subscriptions.drain() {
                            let _ = tx.send(subgraph_response_with_error("Connection closed"));
                            tx.close();
                        }
                        return;
                    }
                    Err(FrameNotParsedToText::None) => {}
                }
            }
            Some(Err(e)) => {
                error!("Dispatch loop WebSocket receiver error: {:?}", e);
                return;
            }
            None => {
                return;
            }
        }
    }
}

/// Guard that cleans up all subscriptions when the message dispatcher is dropped (client-side).
struct DispatcherGuard {
    state: WsStateRef,
}

impl Drop for DispatcherGuard {
    fn drop(&mut self) {
        let err_msg = "Message dispatcher closed";
        for (_, tx) in self.state.borrow_mut().subscriptions.drain() {
            let _ = tx.send(subgraph_response_with_error(err_msg));
            tx.close();
        }
    }
}

async fn send_and_is_closed(sink: WsSink, msg: ws::Message) -> bool {
    let is_close = matches!(msg, ws::Message::Close(_));
    let _ = sink.send(msg).await;
    return is_close;
}

fn handle_text_frame(text: String, state: &WsStateRef) -> Option<ws::Message> {
    let server_msg = match text_to_server_message(&text) {
        Ok(msg) => msg,
        Err(msg) => return Some(msg),
    };

    trace!(msg = ?server_msg, "Received server message");

    match server_msg {
        ServerMessage::ConnectionAck {} => {
            // already received during init, ignore duplicate
            // TODO: consider closing the connection with error,
            //       but it's not that big of a deal since ack is
            //       just a handshake confirmation
            None
        }
        ServerMessage::Next { id, payload } => {
            if let Some(tx) = state.borrow().subscriptions.get(&id) {
                let payload_bytes = Bytes::from(sonic_rs::to_string(&payload).unwrap_or_default());
                let response = match SubgraphResponse::deserialize_from_bytes(payload_bytes) {
                    Ok(response) => response,
                    Err(e) => {
                        tracing::warn!("Failed to deserialize payload: {}", e);
                        subgraph_response_with_error("Failed to deserialize payload")
                    }
                };
                // TODO: should we be strict and close the connection if id did not match any subscription?
                let _ = tx.send(response);
            }
            None
        }
        ServerMessage::Error { id, payload } => {
            if let Some(tx) = state.borrow_mut().subscriptions.remove(&id) {
                let _ = tx.send(SubgraphResponse {
                    errors: Some(payload),
                    ..Default::default()
                });
                tx.close();
            }
            None
        }
        ServerMessage::Complete { id } => {
            if let Some(tx) = state.borrow_mut().subscriptions.remove(&id) {
                tx.close();
            }
            None
        }
        ServerMessage::Ping {} => Some(ClientMessage::pong()),
        ServerMessage::Pong {} => None,
    }
}

fn subgraph_response_with_error(message: &str) -> SubgraphResponse<'static> {
    SubgraphResponse {
        errors: Some(vec![GraphQLError::from_message_and_code(
            message,
            // TODO: define a proper error code for websocket errors
            "WEBSOCKET_ERROR",
        )]),
        ..Default::default()
    }
}

fn text_to_server_message(text: &str) -> Result<ServerMessage, ws::Message> {
    let server_msg: ServerMessage = match sonic_rs::from_str(text) {
        Ok(msg) => msg,
        Err(e) => {
            error!("Failed to parse server message to JSON: {}", e);
            return Err(CloseCode::BadResponse("Invalid message received from server").into());
        }
    };
    Ok(server_msg)
}

// TODO: hella tests
