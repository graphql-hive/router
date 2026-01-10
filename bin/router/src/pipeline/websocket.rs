use futures::future::{select, Either};
use futures::StreamExt;
use hive_router_plan_executor::execution::client_request_details::{
    ClientRequestDetails, JwtRequestDetails, OperationDetails,
};
use hive_router_plan_executor::execution::plan::QueryPlanExecutionResult;
use hive_router_query_planner::state::supergraph_state::OperationKind;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use http::Method;
use ntex::channel::oneshot;
use ntex::service::{fn_factory_with_config, fn_service, fn_shutdown, Service};
use ntex::util::Bytes;
use ntex::web::{self, ws, Error, HttpRequest, HttpResponse};
use ntex::{chain, rt};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, trace, warn};

use crate::pipeline::coerce_variables::coerce_request_variables;
use crate::pipeline::error::PipelineErrorVariant;
use crate::pipeline::execute_pipeline;
use crate::pipeline::execution::{ExposeQueryPlanMode, EXPOSE_QUERY_PLAN_HEADER};
use crate::pipeline::execution_request::ExecutionRequest;
use crate::pipeline::normalize::normalize_request_with_cache;
use crate::pipeline::parser::parse_operation_with_cache;
use crate::pipeline::validation::validate_operation_with_cache;
use crate::schema_state::SchemaState;
use crate::shared_state::RouterSharedState;
use hive_router_plan_executor::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};

pub async fn ws_index(
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
    /// Active subscriptions with their cancellation senders.
    active_subscriptions: HashMap<String, mpsc::Sender<()>>,
}

impl WsState {
    fn new() -> Self {
        Self {
            last_heartbeat: Instant::now(),
            active_subscriptions: HashMap::new(),
        }
    }
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

    let state = Rc::new(RefCell::new(WsState::new()));

    let (tx, rx) = oneshot::channel();

    rt::spawn(heartbeat(state.clone(), sink.clone(), rx));

    let service = fn_service(move |frame| {
        let sink = sink.clone();
        let state = state.clone();
        let schema_state = schema_state.clone();
        let shared_state = shared_state.clone();
        async move {
            let item = match frame {
                ws::Frame::Text(text) => {
                    handle_text_frame(text, sink, state, &schema_state, &shared_state).await
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
    state: Rc<RefCell<WsState>>,
    schema_state: &Arc<SchemaState>,
    shared_state: &Arc<RouterSharedState>,
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
        ClientMessage::Subscribe {
            ref id,
            mut payload,
        } => {
            let maybe_supergraph = schema_state.current_supergraph();
            let supergraph = match maybe_supergraph.as_ref() {
                Some(supergraph) => supergraph,
                None => {
                    warn!(
                        "No supergraph available yet, unable to process client subscribe message"
                    );
                    return Some(ServerMessage::error(
                        id,
                        &vec![GraphQLError::from_message_and_extensions(
                            "No supergraph available yet".to_string(),
                            GraphQLErrorExtensions::new_from_code("SERVICE_UNAVAILABLE"),
                        )],
                    ));
                }
            };

            let parser_payload = match parse_operation_with_cache(shared_state, &payload).await {
                Ok(payload) => payload,
                Err(err) => return Some(err.into_server_message(id)),
            };

            if let Err(err) = validate_operation_with_cache(
                supergraph,
                schema_state,
                shared_state,
                &parser_payload,
            )
            .await
            {
                return Some(err.into_server_message(id));
            }

            let normalize_payload = match normalize_request_with_cache(
                supergraph,
                schema_state,
                &payload,
                &parser_payload,
            )
            .await
            {
                Ok(payload) => payload,
                Err(err) => return Some(err.into_server_message(id)),
            };

            let variable_payload =
                match coerce_request_variables(supergraph, &mut payload, &normalize_payload) {
                    Ok(payload) => payload,
                    Err(err) => return Some(err.into_server_message(id)),
                };

            let query_plan_cancellation_token =
                CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

            // TODO: extract from connection init payload and extensions of payload
            let headers = ntex::http::HeaderMap::new();

            // TODO: extract from connection init payload and extensions of payload
            let jwt_request_details = JwtRequestDetails::Unauthenticated;

            let client_request_details = ClientRequestDetails {
                method: &Method::POST,
                url: &http::Uri::from_static("/graphql"),
                headers: &headers,
                operation: OperationDetails {
                    name: normalize_payload.operation_for_plan.name.as_deref(),
                    kind: match normalize_payload.operation_for_plan.operation_kind {
                        Some(OperationKind::Query) => "query",
                        Some(OperationKind::Mutation) => "mutation",
                        Some(OperationKind::Subscription) => "subscription",
                        None => "query",
                    },
                    query: &payload.query,
                },
                jwt: &jwt_request_details,
            };

            let mut expose_query_plan = ExposeQueryPlanMode::No;
            if shared_state.router_config.query_planner.allow_expose {
                if let Some(expose_qp_header) = headers.get(&EXPOSE_QUERY_PLAN_HEADER) {
                    let str_value = expose_qp_header.to_str().unwrap_or_default().trim();
                    match str_value {
                        "true" => expose_query_plan = ExposeQueryPlanMode::Yes,
                        "dry-run" => expose_query_plan = ExposeQueryPlanMode::DryRun,
                        _ => {}
                    }
                }
            }

            match execute_pipeline(
                &query_plan_cancellation_token,
                &client_request_details,
                &normalize_payload,
                &variable_payload,
                &expose_query_plan,
                supergraph,
                shared_state,
                schema_state,
            )
            .await
            {
                Ok(QueryPlanExecutionResult::Single(response)) => {
                    let _ = sink.send(ServerMessage::next(id, &response.body)).await;
                    Some(ServerMessage::complete(id))
                }
                Ok(QueryPlanExecutionResult::Stream(response)) => {
                    let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);

                    state
                        .borrow_mut()
                        .active_subscriptions
                        .insert(id.to_string(), cancel_tx);

                    let mut stream = response.body;
                    let id_string = id.to_string();

                    loop {
                        tokio::select! {
                            maybe_item = stream.next() => {
                                match maybe_item {
                                    Some(body) => {
                                        let _ = sink.send(ServerMessage::next(&id_string, &body)).await;
                                    }
                                    None => {
                                        break; // completed
                                    }
                                }
                            }
                            _ = cancel_rx.recv() => {
                                trace!(id = %id_string, "Subscription cancelled by client");
                                break; // cancelled
                            }
                        }
                    }

                    state.borrow_mut().active_subscriptions.remove(&id_string);

                    Some(ServerMessage::complete(id))
                }
                Err(err) => Some(err.into_server_message(id)),
            }
        }
        _ => {
            todo!();
        }
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
enum ServerMessage<'a> {
    Next {
        id: &'a str,
        payload: &'a sonic_rs::Value,
    },
    Error {
        id: &'a str,
        payload: &'a [GraphQLError],
    },
    Complete {
        id: &'a str,
    },
}

impl ServerMessage<'_> {
    pub fn next(id: &str, body: &[u8]) -> ws::Message {
        let payload = match sonic_rs::from_slice(body) {
            Ok(value) => value,
            Err(err) => {
                error!("Failed to serialize plan execution output body: {}", err);
                return ws::Message::Close(Some(ws::CloseReason {
                    code: ntex::ws::CloseCode::from(4500),
                    description: Some("Internal Server Error".into()),
                }));
            }
        };
        ServerMessage::Next {
            id,
            payload: &payload,
        }
        .into()
    }
    pub fn error(id: &str, payload: &[GraphQLError]) -> ws::Message {
        ServerMessage::Error { id, payload }.into()
    }
    pub fn complete(id: &str) -> ws::Message {
        ServerMessage::Complete { id }.into()
    }
}

impl From<ServerMessage<'_>> for ws::Message {
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

impl PipelineErrorVariant {
    fn into_server_message(&self, id: &str) -> ws::Message {
        let code = self.graphql_error_code();
        let message = self.graphql_error_message();

        let graphql_error = GraphQLError::from_message_and_extensions(
            message,
            GraphQLErrorExtensions::new_from_code(code),
        );

        ServerMessage::error(id, &vec![graphql_error])
    }
}
