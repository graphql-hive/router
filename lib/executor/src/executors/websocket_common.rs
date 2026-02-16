use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration, time::Instant};

use futures::future::{select, Either};
use ntex::{
    channel::oneshot,
    util::Bytes,
    ws::{self, WsSink},
};
use tracing::{debug, error, warn};

use super::graphql_transport_ws::{CloseCode, ConnectionInitPayload};

/// Shared WebSocket state for both client and server implementations.
pub struct WsState<T> {
    /// The moment of the last heartbeat received from the peer. This is used
    /// to detect stale connections and drop them on timeout.
    pub last_heartbeat: Instant,
    /// Indicates whether the handshake message has been received.
    ///
    /// - Server received ConnectionInit
    /// - Client received ConnectionAck
    ///
    /// This flag is used to enforce the peer cant send multiple handshake messages.
    ///
    /// Do not confuse it with acknowledged_tx.
    pub handshake_received: bool,
    /// Sender to indicate that the handshake has been completed and that
    /// the timeout task should cancel.
    ///
    /// When `None`, the handshake has been completed.
    pub acknowledged_tx: Option<oneshot::Sender<()>>,
    /// Payload from the connection init message.
    pub init_payload: Option<ConnectionInitPayload>,
    /// Active subscriptions in the WebSocket connection.
    ///
    /// - Server subscriptions map subscription IDs to their cancellation sender.
    /// - Client subscriptions map subscription IDs to their response senders.
    pub subscriptions: HashMap<String, T>,
}

impl<T> WsState<T> {
    pub fn new(acknowledged_tx: oneshot::Sender<()>) -> Self {
        Self {
            last_heartbeat: Instant::now(),
            handshake_received: false,
            acknowledged_tx: Some(acknowledged_tx),
            init_payload: None,
            subscriptions: HashMap::new(),
        }
    }

    pub fn is_acknowledged(&self) -> bool {
        self.acknowledged_tx.is_none()
    }

    /// Checks if the connection has been acknowledged; if not, returns a close
    /// frame for the peer.
    pub fn check_acknowledged(&self) -> Option<ws::Message> {
        if self.is_acknowledged() {
            None
        } else {
            Some(CloseCode::Unauthorized.into())
        }
    }

    pub fn complete_handshake(&mut self) {
        if let Some(tx) = self.acknowledged_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Heartbeat ping interval.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// Peer response to heartbeat timeout.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);
/// Handshake message received timeout.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Ping peer every heartbeat interval.
pub async fn heartbeat<T>(
    state: Rc<RefCell<WsState<T>>>,
    sink: WsSink,
    mut stop_rx: oneshot::Receiver<()>,
) {
    loop {
        match select(
            Box::pin(ntex::time::sleep(HEARTBEAT_INTERVAL)),
            &mut stop_rx,
        )
        .await
        {
            Either::Left(_) => {
                if Instant::now().duration_since(state.borrow().last_heartbeat) > HEARTBEAT_TIMEOUT
                {
                    debug!("WebSocket heartbeat timeout, closing connection");
                    let _ = sink
                        .send(ws::Message::Close(Some(
                            // client is violating the WebSocket protocol by not responding
                            // to the PING frames with PONG frames as required by the spec,
                            // so we use the "Protocol Error" to close the connection
                            ws::CloseCode::Protocol.into(),
                        )))
                        .await;
                    return;
                }
                if sink
                    .send(ws::Message::Ping(Bytes::default()))
                    .await
                    .is_err()
                {
                    warn!("Failed to send WebSocket heartbeat ping, stopping heartbeat task");
                    return;
                }
            }
            Either::Right(_) => return,
        }
    }
}

/// Monitor handshake timeout and close connection if handshake not completed in time. Uses the provided
/// close code to close the connection on timeout.
pub async fn handshake_timeout<T>(
    state: Rc<RefCell<WsState<T>>>,
    sink: WsSink,
    mut stop_rx: oneshot::Receiver<()>,
    timeout_close_code: CloseCode,
) {
    match select(Box::pin(ntex::time::sleep(HANDSHAKE_TIMEOUT)), &mut stop_rx).await {
        Either::Left(_) => {
            // handshake_received should always be here false, but double check
            // just to avoid any potential race conditions
            if !state.borrow().handshake_received {
                debug!("WebSocket handshake timeout, closing connection");
                let _ = sink.send(timeout_close_code.into()).await;
            }
        }
        Either::Right(_) => {
            // cancelled, handshake was completed
        }
    }
}

/// The frame was handled but not parsed to text. In this case the
pub enum FrameNotParsedToText {
    /// Not parsed to text but there is a message to send back to the peer.
    Message(ws::Message),
    /// Not parsed to text and nothing to send back to the peer. This happens
    /// when receiving a pong or other non-text frames that don't require a response.
    None,
    /// Connection was closed (Close frame received), nothing to send back to the peer.
    Closed,
}

/// Parse WebSocket frame to text, returning messages to send back on non-parsed-text frames.
pub fn parse_frame_to_text<T>(
    frame: ws::Frame,
    state: &Rc<RefCell<WsState<T>>>,
) -> Result<String, FrameNotParsedToText> {
    match frame {
        ws::Frame::Text(text) => {
            match String::from_utf8(text.to_vec()) {
                Ok(s) => Ok(s),
                Err(e) => {
                    error!("Invalid UTF-8 in WebSocket message: {}", e);
                    Err(FrameNotParsedToText::Message(ws::Message::Close(Some(
                        // this one is not in the CloseCode enum because it's an internal WebSocket
                        // transport error that has nothing to do with GraphQL over WebSockets
                        ws::CloseReason {
                            code: ws::CloseCode::Unsupported,
                            description: Some("Invalid UTF-8 in message".into()),
                        },
                    ))))
                }
            }
        }
        ws::Frame::Ping(data) => {
            state.borrow_mut().last_heartbeat = Instant::now();
            Err(FrameNotParsedToText::Message(ws::Message::Pong(data)))
        }
        ws::Frame::Pong(_) => {
            state.borrow_mut().last_heartbeat = Instant::now();
            Err(FrameNotParsedToText::None)
        }
        // we don't support binary frames
        ws::Frame::Binary(_) => Err(FrameNotParsedToText::Message(ws::Message::Close(Some(
            ws::CloseReason {
                // this one is not in the CloseCode enum because it's an internal WebSocket
                // transport error that has nothing to do with GraphQL over WebSockets
                code: ws::CloseCode::Unsupported,
                description: Some("Unsupported message type".into()),
            },
        )))),
        ws::Frame::Close(reason) => {
            if let Some(close_reason) = reason {
                debug!(
                    code = ?close_reason.code,
                    description = ?close_reason.description,
                    "WebSocket connection closed",
                );
            }
            Err(FrameNotParsedToText::Closed)
        }
        _ => Err(FrameNotParsedToText::None),
    }
}
