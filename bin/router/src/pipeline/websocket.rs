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
use ntex::http::{header::HeaderName, header::HeaderValue, HeaderMap};
use ntex::service::{fn_factory_with_config, fn_service, fn_shutdown, Service};
use ntex::util::Bytes;
use ntex::web::{self, ws, Error, HttpRequest, HttpResponse};
use ntex::{chain, rt};
use serde::{Deserialize, Serialize};
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, trace, warn};

use crate::jwt::context::JwtRequestContext;
use crate::jwt::errors::JwtError;
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
    /// to detect stale clients and drop the connection on timeout.
    last_heartbeat: Instant,
    /// Indicates whether the connection init message has been received.
    ///
    /// This flag is only used to enforce the client cant send multiple connection
    /// init messages.
    ///
    /// Do not confuse it with acknowledged_tx.
    connection_init_received: bool,
    /// Sender to indicate that the connection init has been received and that
    /// the timeout task should cancel.
    ///
    /// When `None`, the connection init message has been received, validated and
    /// therefore the connection acknowledged.
    acknowledged_tx: Option<oneshot::Sender<()>>,
    /// Current headers from the client (from connection init or last message).
    ///
    /// Not to be confused with http headers, these are NOT http headers, these
    /// are either the map sent in the connection init message payload or the headers
    /// property in the extensions of the subscribe message payload (graphql execution request).
    ///
    /// They are considered "current" because they can be updated by the client
    /// on each subscribe message by providing new headers in the extensions.
    current_headers: HeaderMap,
    /// Active subscriptions with their cancellation senders.
    active_subscriptions: HashMap<String, mpsc::Sender<()>>,
}

impl WsState {
    fn new(acknowledged: oneshot::Sender<()>) -> Self {
        Self {
            last_heartbeat: Instant::now(),
            connection_init_received: false,
            acknowledged_tx: Some(acknowledged),
            current_headers: HeaderMap::new(),
            active_subscriptions: HashMap::new(),
        }
    }
    /// Checks if the connection has been acknowledged; if not, returns a close
    /// frame for the client.
    fn check_acknowledged(&self) -> Option<ws::Message> {
        if self.acknowledged_tx.is_none() {
            Some(CloseCode::Unauthorized.into())
        } else {
            None
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

/// Connection init message received timeout.
const CONNECTION_INIT_TIMEOUT: Duration = Duration::from_secs(10);
/// Monitor connection init timeout and close connection if init not received in time.
async fn connection_init_timeout(
    state: Rc<RefCell<WsState>>,
    sink: web::ws::WsSink,
    mut rx: oneshot::Receiver<()>,
) {
    match select(
        Box::pin(ntex::time::sleep(CONNECTION_INIT_TIMEOUT)),
        &mut rx,
    )
    .await
    {
        Either::Left(_) => {
            // connection_init_received should always be here false, but double check
            // just to avoid any potential race conditions (see handling of the connection
            // init message below)
            if !state.borrow().connection_init_received {
                debug!("WebSocket connection init timeout, closing connection");
                let _ = sink.send(CloseCode::ConnectionInitTimeout.into()).await;
            }
        }
        Either::Right(_) => {
            // cancelled, connection_init was received
        }
    }
}

async fn ws_service(
    sink: ws::WsSink,
    schema_state: Arc<SchemaState>,
    shared_state: Arc<RouterSharedState>,
) -> Result<impl Service<ws::Frame, Response = Option<ws::Message>, Error = io::Error>, web::Error>
{
    debug!("WebSocket connection opened");

    let (heartbeat_tx, heartbeat_rx) = oneshot::channel();
    let (init_timeout_tx, _) = oneshot::channel();
    let (acknowledged_tx, acknowledged_rx) = oneshot::channel();

    let state = Rc::new(RefCell::new(WsState::new(acknowledged_tx)));

    rt::spawn(heartbeat(state.clone(), sink.clone(), heartbeat_rx));
    rt::spawn(connection_init_timeout(
        state.clone(),
        sink.clone(),
        acknowledged_rx,
    ));

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
                // heartbeat
                web::ws::Frame::Ping(msg) => {
                    state.borrow_mut().last_heartbeat = Instant::now();
                    Some(web::ws::Message::Pong(msg))
                }
                web::ws::Frame::Pong(_) => {
                    state.borrow_mut().last_heartbeat = Instant::now();
                    None
                }
                // we don't support binary frames
                ws::Frame::Binary(_) => Some(ws::Message::Close(Some(ws::CloseReason {
                    // this one is not in the CloseCode enum because it's an internal WebSocket
                    // transport error that has nothing to do with GraphQL over WebSockets
                    code: ws::CloseCode::Unsupported,
                    description: Some("Unsupported message type".into()),
                }))),
                // closing connection. we cant send any more message so we just None
                ws::Frame::Close(msg) => {
                    if let Some(close_reason) = msg {
                        debug!(
                            code = ?close_reason.code,
                            description = ?close_reason.description,
                            "WebSocket connection closed",
                        );
                    }
                    // cancel all active subscriptions on close allowing a clean shutdown.
                    // clearing the map will drop all the senders, which will
                    // in turn cancel all active subscription streams
                    state.borrow_mut().active_subscriptions.clear();
                    None
                }
                // ignore other frames (should not match)
                _ => None,
            };
            Ok(item)
        }
    });

    let on_shutdown = fn_shutdown(move || {
        // stop heartbeat and init timeout tasks on shutdown
        let _ = heartbeat_tx.send(());
        let _ = init_timeout_tx.send(());
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
            return Some(ws::Message::Close(Some(
                // this one is not in the CloseCode enum because it's an internal WebSocket
                // transport error that has nothing to do with GraphQL over WebSockets
                ws::CloseReason {
                    code: ws::CloseCode::Unsupported,
                    description: Some("Invalid UTF-8 in message".into()),
                },
            )));
        }
    };

    let client_msg: ClientMessage = match sonic_rs::from_str(&text) {
        Ok(msg) => msg,
        Err(e) => {
            error!("Failed to parse client message to JSON: {}", e);
            return Some(CloseCode::BadRequest("Invalid message received").into());
        }
    };

    trace!(msg = ?client_msg, "Received client message");

    match client_msg {
        ClientMessage::ConnectionInit { payload } => {
            if state.borrow().connection_init_received {
                return Some(CloseCode::TooManyInitialisationRequests.into());
            }
            state.borrow_mut().connection_init_received = true;

            // cancel the connection init timeout since we received the init message
            if let Some(tx) = state.borrow_mut().acknowledged_tx.take() {
                let _ = tx.send(());
            }

            let _ = sink.send(ServerMessage::ack(None).into()).await;

            debug!("Connection acknowledged");

            let header_map = parse_headers_from_connection_init_payload(payload);
            if !header_map.is_empty() {
                trace!(headers = ?header_map, "Connection init message contains headers in the payload");
            } else {
                trace!("Connection init message does not contain headers in the payload");
            }
            state.borrow_mut().current_headers = header_map;

            None
        }
        ClientMessage::Ping {} => {
            // respond with pong always, regardless of acknowledged state
            // the client should be able to use subprotocol pings/pongs to check liveness
            Some(ServerMessage::pong())
        }
        ClientMessage::Subscribe { id, mut payload } => {
            state.borrow().check_acknowledged()?;

            if state.borrow().active_subscriptions.contains_key(&id) {
                return Some(CloseCode::SubscriberAlreadyExists(id).into());
            }

            let maybe_supergraph = schema_state.current_supergraph();
            let supergraph = match maybe_supergraph.as_ref() {
                Some(supergraph) => supergraph,
                None => {
                    warn!(
                        "No supergraph available yet, unable to process client subscribe message"
                    );
                    return Some(ServerMessage::error(
                        &id,
                        &vec![GraphQLError::from_message_and_extensions(
                            "No supergraph available yet".to_string(),
                            GraphQLErrorExtensions::new_from_code("SERVICE_UNAVAILABLE"),
                        )],
                    ));
                }
            };

            let parser_payload = match parse_operation_with_cache(shared_state, &payload).await {
                Ok(payload) => payload,
                Err(err) => return Some(err.into_server_message(&id)),
            };

            if let Err(err) = validate_operation_with_cache(
                supergraph,
                schema_state,
                shared_state,
                &parser_payload,
            )
            .await
            {
                return Some(err.into_server_message(&id));
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
                Err(err) => return Some(err.into_server_message(&id)),
            };

            let variable_payload =
                match coerce_request_variables(supergraph, &mut payload, &normalize_payload) {
                    Ok(payload) => payload,
                    Err(err) => return Some(err.into_server_message(&id)),
                };

            let query_plan_cancellation_token =
                CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

            let headers = parse_headers_from_extensions(payload.extensions.as_ref());

            // TODO: should we update the current headers in state?

            let mut jwt_context: Option<JwtRequestContext> = None;
            if let Some(jwt) = &shared_state.jwt_auth_runtime {
                match jwt
                    .verify_headers(&headers, &shared_state.jwt_claims_cache)
                    .await
                {
                    Ok(maybe_jwt_context) => {
                        if let Some(ctx) = maybe_jwt_context {
                            jwt_context = Some(ctx);
                        };
                    }
                    Err(err) => {
                        let _ = sink.send(err.into_server_message(&id)).await;
                        // we report error as graphql error, but we also close the
                        // connection since we're dealing with auth so let's be safe
                        return Some(err.into_close_message());
                    }
                }
            }
            let jwt_claims = match &jwt_context {
                Some(jwt_context) => match jwt_context.get_claims_value() {
                    Ok(claims) => Some(claims),
                    Err(e) => {
                        return Some(
                            PipelineErrorVariant::JwtForwardingError(e).into_close_message(),
                        )
                    }
                },
                None => None,
            };
            let jwt_request_details = match (&jwt_context, &jwt_claims) {
                (Some(jwt_context), Some(claims)) => JwtRequestDetails::Authenticated {
                    token: jwt_context.token_raw.as_str(),
                    prefix: jwt_context.token_prefix.as_deref(),
                    scopes: jwt_context.extract_scopes(),
                    claims,
                },
                _ => JwtRequestDetails::Unauthenticated,
            };

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
                    let _ = sink.send(ServerMessage::next(&id, &response.body)).await;
                    Some(ServerMessage::complete(&id))
                }
                Ok(QueryPlanExecutionResult::Stream(response)) => {
                    // we use mpsc::channel(1) instead of oneshot because oneshot::Receiver
                    // is consumed on first await, which doesn't work in tokio::select! loops that
                    // need to poll the receiver multiple times across iterations
                    let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);

                    state
                        .borrow_mut()
                        .active_subscriptions
                        .insert(id.clone(), cancel_tx);

                    // automatically remove the subscription from active_subscriptions when dropped
                    let _guard = SubscriptionGuard {
                        state: state.clone(),
                        id: id.clone(),
                    };

                    let mut stream = response.body;
                    let mut cancelled = false;

                    trace!(id = %id, "Subscription started");

                    let id_for_loop = id.clone();
                    loop {
                        tokio::select! {
                            maybe_item = stream.next() => {
                                match maybe_item {
                                    Some(body) => {
                                        let _ = sink.send(ServerMessage::next(&id_for_loop, &body)).await;
                                    }
                                    None => {
                                        break; // completed
                                    }
                                }
                            }
                            _ = cancel_rx.recv() => {
                                cancelled = true;
                                break; // cancelled
                            }
                        }
                    }

                    if cancelled {
                        trace!(id = %id, "Subscription cancelled");
                        None
                    } else {
                        trace!(id = %id, "Subscription completed");
                        Some(ServerMessage::complete(&id))
                    }
                }
                Err(err) => Some(err.into_server_message(&id)),
            }
        }
        ClientMessage::Complete { id } => {
            state.borrow().check_acknowledged()?;

            if let Some(cancel_tx) = state.borrow_mut().active_subscriptions.remove(&id) {
                trace!(id = %id, "Client requested subscription cancellation");
                let _ = cancel_tx.try_send(());
            }
            None
        }
    }
}

/// Ensure a subscription is removed from active_subscriptions when dropped,
/// regardless of how the subscription stream itself ends (normal completion,
/// cancellation, panic (hopefully not), or future being dropped at an await point).
struct SubscriptionGuard {
    state: Rc<RefCell<WsState>>,
    id: String,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        self.state
            .borrow_mut()
            .active_subscriptions
            .remove(&self.id);
        trace!(id = %self.id, "Subscription removed from active subscritpions");
    }
}

/// Parses headers from a sonic_rs::Object into a HeaderMap. Only stringifiable
/// values are included; nulls, arrays, objects and other non-primitive values
/// are ignored.
fn parse_headers_from_object(headers_obj: &sonic_rs::Object) -> HeaderMap {
    let mut header_map = HeaderMap::new();

    for (key, value) in headers_obj {
        let value_str = if let Some(s) = value.as_str() {
            Some(s.to_string())
        } else if let Some(b) = value.as_bool() {
            Some(b.to_string())
        } else if let Some(i) = value.as_i64() {
            Some(i.to_string())
        } else if let Some(f) = value.as_f64() {
            Some(f.to_string())
        } else {
            None // ignore nulls, arrays, objects and whatever else
        };

        if let Some(val_str) = value_str {
            if let (Ok(name), Ok(val)) = (HeaderName::try_from(key), HeaderValue::try_from(val_str))
            {
                header_map.insert(name, val);
            }
        }
    }

    header_map
}

fn parse_headers_from_connection_init_payload(payload: Option<ConnectionInitPayload>) -> HeaderMap {
    let mut header_map = HeaderMap::new();
    if let Some(payload) = payload {
        if let Some(headers_prop) = payload.fields.get("headers") {
            if let Some(headers_obj) = headers_prop.as_object() {
                header_map = parse_headers_from_object(headers_obj);
            }
        }
    }
    header_map
}

fn parse_headers_from_extensions(
    extensions: Option<&HashMap<String, sonic_rs::Value>>,
) -> HeaderMap {
    let mut header_map = HeaderMap::new();
    if let Some(ext) = extensions {
        if let Some(headers_value) = ext.get("headers") {
            if let Some(headers_obj) = headers_value.as_object() {
                header_map = parse_headers_from_object(headers_obj);
            }
        }
    }
    header_map
}

enum CloseCode {
    ConnectionInitTimeout,
    TooManyInitialisationRequests,
    Unauthorized,
    Forbidden(String),
    BadRequest(&'static str),
    SubscriberAlreadyExists(String),
    InternalServerError(Option<String>),
}

impl From<CloseCode> for ws::Message {
    fn from(msg: CloseCode) -> Self {
        match msg {
            CloseCode::ConnectionInitTimeout => ws::Message::Close(Some(ws::CloseReason {
                code: ws::CloseCode::from(4408),
                description: Some("Connection initialisation timeout".into()),
            })),
            CloseCode::TooManyInitialisationRequests => ws::Message::Close(Some(ws::CloseReason {
                code: ws::CloseCode::from(4429),
                description: Some("Too many initialisation requests".into()),
            })),
            CloseCode::Unauthorized => ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4401),
                description: Some("Unauthorized".into()),
            })),
            CloseCode::Forbidden(reason) => ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4403),
                description: Some(reason),
            })),
            CloseCode::BadRequest(reason) => ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4400),
                description: Some(reason.into()),
            })),
            CloseCode::SubscriberAlreadyExists(id) => ws::Message::Close(Some(ws::CloseReason {
                code: ws::CloseCode::from(4409),
                description: Some(format!("Subscriber for {id} already exists")),
            })),
            CloseCode::InternalServerError(reason) => ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4500),
                description: reason.or(Some("Internal Server Error".into())),
            })),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    ConnectionInit {
        payload: Option<ConnectionInitPayload>,
    },
    Ping {},
    // TODO: implement digesting pongs from client using subprotocol ping/pong
    Subscribe {
        id: String,
        payload: ExecutionRequest,
    },
    Complete {
        id: String,
    },
}

/// The connection init message payload MUST be a map of string to arbitrary JSON
/// values as per the spec. We represent this as a HashMap<String, Value> and use
/// serde(flatten) to capture all fields for easier parsing to headers later.
#[derive(Deserialize, Debug)]
struct ConnectionInitPayload {
    #[serde(flatten)]
    fields: HashMap<String, sonic_rs::Value>,
}

#[derive(Serialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage<'a> {
    ConnectionAck {
        #[serde(skip_serializing_if = "Option::is_none")]
        payload: Option<sonic_rs::Value>,
    },
    Pong {},
    // TODO: implement pinging from server using subprotocol ping/pong
    Next {
        id: &'a str,
        payload: sonic_rs::Value,
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
    pub fn ack(payload: Option<sonic_rs::Value>) -> ws::Message {
        ServerMessage::ConnectionAck { payload }.into()
    }
    pub fn pong() -> ws::Message {
        ServerMessage::Pong {}.into()
    }
    pub fn next(id: &str, body: &[u8]) -> ws::Message {
        let payload = match sonic_rs::from_slice(body) {
            Ok(value) => value,
            Err(err) => {
                error!("Failed to serialize plan execution output body: {}", err);
                return CloseCode::InternalServerError(None).into();
            }
        };
        ServerMessage::Next { id, payload }.into()
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
                CloseCode::InternalServerError(None).into()
            }
        }
    }
}

// NOTE: no `From` trait because it can into ws message and ws closecode but both are ws::Message
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
    fn into_close_message(&self) -> ws::Message {
        CloseCode::InternalServerError(Some(self.graphql_error_code().to_string())).into()
    }
}

// NOTE: no `From` trait because it can into ws message and ws closecode but both are ws::Message
impl JwtError {
    fn into_server_message(&self, id: &str) -> ws::Message {
        ServerMessage::error(id, &vec![self.into()])
    }
    fn into_close_message(&self) -> ws::Message {
        CloseCode::Forbidden(self.error_code().to_string()).into()
    }
}

#[cfg(test)]
mod tests {
    use sonic_rs::json;

    use super::*;

    #[test]
    fn should_parse_headers_from_object() {
        let headers_json = json!({
            "authorization": "Bearer token123",
            "x-custom-header": "custom-value",
            "x-number": 42,
            "x-bool": true,
            "x-float": 3.14
        });

        let headers_obj = headers_json.as_object().expect("Failed to get object");

        let headers = parse_headers_from_object(headers_obj);

        assert_eq!(
            headers.get("authorization").unwrap().to_str().unwrap(),
            "Bearer token123"
        );
        assert_eq!(
            headers.get("x-custom-header").unwrap().to_str().unwrap(),
            "custom-value"
        );
        assert_eq!(headers.get("x-number").unwrap().to_str().unwrap(), "42");
        assert_eq!(headers.get("x-bool").unwrap().to_str().unwrap(), "true");
        assert_eq!(headers.get("x-float").unwrap().to_str().unwrap(), "3.14");
    }
}
