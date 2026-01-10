use futures::future::{select, Either};
use ntex::channel::oneshot;
use ntex::service::{fn_factory_with_config, fn_service, fn_shutdown, Service};
use ntex::util::Bytes;
use ntex::web::{self, ws, Error, HttpRequest, HttpResponse};
use ntex::{chain, rt};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, trace, warn};

use crate::pipeline::execution_request::ExecutionRequest;
use crate::schema_state::SchemaState;
use crate::shared_state::RouterSharedState;
use hive_router_plan_executor::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};

pub async fn graphql_ws_handler(
    req: HttpRequest,
    schema_state: web::types::State<Arc<SchemaState>>,
    shared_state: web::types::State<Arc<RouterSharedState>>,
) -> Result<HttpResponse, Error> {
    let schema_state = schema_state.get_ref().clone();
    let shared_state = shared_state.get_ref().clone();

    ws::start(
        req,
        fn_factory_with_config(move |sink: ws::WsSink| {
            let schema_state = schema_state.clone();
            let shared_state = shared_state.clone();
            async move { ws_service(sink, schema_state, shared_state).await }
        }),
    )
    .await
}

struct WsState {
    /// The moment of the last heartbeat received from the client. This is used
    /// to detect client timeouts and drop the connection on timeout.
    last_heartbeat: Instant,
}

/// Heartbeat ping interval.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// Client response to heartbeat timeout.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Ping client every heartbeat interval.
async fn heartbeat(
    state: Rc<RefCell<WsState>>,
    sink: web::ws::WsSink,
    mut rx: oneshot::Receiver<()>,
) {
    loop {
        match select(Box::pin(ntex::time::sleep(HEARTBEAT_INTERVAL)), &mut rx).await {
            Either::Left(_) => {
                if Instant::now().duration_since(state.borrow().last_heartbeat) > CLIENT_TIMEOUT {
                    debug!("WebSocket client heartbeat timeout");
                    return;
                }
                if sink
                    .send(web::ws::Message::Ping(Bytes::default()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Either::Right(_) => return,
        }
    }
}

async fn ws_service(
    sink: ws::WsSink,
    schema_state: Arc<SchemaState>,
    shared_state: Arc<RouterSharedState>,
) -> Result<impl Service<ws::Frame, Response = Option<ws::Message>, Error = io::Error>, web::Error>
{
    debug!("WebSocket connection established");

    let state = Rc::new(RefCell::new(WsState {
        last_heartbeat: Instant::now(),
    }));

    let (tx, rx) = oneshot::channel();

    rt::spawn(heartbeat(state.clone(), sink.clone(), rx));

    let sink = sink.clone();
    let state = state.clone();
    let schema_state = schema_state.clone();
    let shared_state = shared_state.clone();

    let service = fn_service(move |frame| {
        let sink = sink.clone();
        let state = state.clone();
        let schema_state = schema_state.clone();
        let shared_state = shared_state.clone();
        async move {
            let item = match frame {
                ws::Frame::Text(text) => {
                    handle_text_frame(text, sink, schema_state, shared_state).await
                }
                // we don't support binary frames
                // TODO: should we drop the connection altogether?
                ws::Frame::Binary(_) => None,
                // heartbeat
                web::ws::Frame::Ping(msg) => {
                    state.borrow_mut().last_heartbeat = Instant::now();
                    Some(web::ws::Message::Pong(msg))
                }
                web::ws::Frame::Pong(_) => {
                    state.borrow_mut().last_heartbeat = Instant::now();
                    None
                }
                // closing connection. we cant send any more message so we just None
                ws::Frame::Close(_) => None,
                // ignore other frames (should not match)
                _ => None,
            };
            Ok(item)
        }
    });

    let on_shutdown = fn_shutdown(move || {
        // stop heartbeat task on shutdown
        let _ = tx.send(());
    });

    Ok(chain(service).and_then(on_shutdown))
}

async fn handle_text_frame(
    text: Bytes,
    sink: ws::WsSink,
    schema_state: Arc<SchemaState>,
    _shared_state: Arc<RouterSharedState>,
) -> Option<ws::Message> {
    let text = match String::from_utf8(text.to_vec()) {
        Ok(s) => s,
        Err(e) => {
            error!("Invalid UTF-8 in WebSocket message: {}", e);
            return Some(ws::Message::Close(Some(ws::CloseReason {
                code: ws::CloseCode::Invalid,
                description: Some("Invalid UTF-8 in message".into()),
            })));
        }
    };

    let client_msg: ClientMessage = match sonic_rs::from_str(&text) {
        Ok(msg) => msg,
        Err(e) => {
            error!("Failed to parse client message to JSON: {}", e);
            return Some(ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4400),
                description: Some("Invalid message received".into()),
            })));
        }
    };

    trace!("Received client message: {:?}", client_msg);

    match client_msg {
        ClientMessage::Subscribe { id, payload } => {
            let maybe_supergraph = schema_state.current_supergraph();
            let supergraph = match maybe_supergraph.as_ref() {
                Some(supergraph) => supergraph,
                None => {
                    warn!(
                        "No supergraph available yet, unable to process client subscribe message"
                    );
                    return Some(
                        ServerMessage::Error {
                            id: id.clone(),
                            payload: vec![GraphQLError::from_message_and_extensions(
                                "No supergraph available yet".to_string(),
                                GraphQLErrorExtensions::new_from_code("SERVICE_UNAVAILABLE"),
                            )],
                        }
                        .into(),
                    );
                }
            };
            Some(ws::Message::Text("TODO".into()))
        }
        _ => Some(ws::Message::Text("TODO".into())),
    }
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMessage {
    Subscribe {
        id: String,
        payload: ExecutionRequest,
    },
    Complete {
        id: String,
    },
}

#[derive(Serialize, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ServerMessage {
    Next {
        id: String,
        // TODO: define a proper payload type?
        payload: serde_json::Value,
    },
    Error {
        id: String,
        payload: Vec<GraphQLError>,
    },
    Complete {
        id: String,
    },
}

impl From<ServerMessage> for ws::Message {
    fn from(msg: ServerMessage) -> Self {
        match sonic_rs::to_string(&msg) {
            Ok(text) => ws::Message::Text(text.into()),
            Err(e) => {
                error!("Failed to serialize server message to JSON: {}", e);
                ws::Message::Close(Some(ws::CloseReason {
                    code: ntex::ws::CloseCode::from(4500),
                    description: Some("Internal Server Error".into()),
                }))
            }
        }
    }
}
