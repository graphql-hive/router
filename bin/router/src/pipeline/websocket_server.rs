use futures::StreamExt;
use hive_router_plan_executor::execution::client_request_details::{
    ClientRequestDetails, JwtRequestDetails, OperationDetails,
};
use hive_router_plan_executor::execution::plan::QueryPlanExecutionResult;
use hive_router_plan_executor::executors::graphql_transport_ws::{
    ClientMessage, CloseCode, ConnectionInitPayload, ServerMessage, WS_SUBPROTOCOL,
};
use hive_router_plan_executor::executors::websocket_common::{
    handshake_timeout, heartbeat, parse_frame_to_text, FrameNotParsedToText, WsState,
};
use hive_router_query_planner::state::supergraph_state::OperationKind;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use http::Method;
use ntex::channel::oneshot;
use ntex::http::{header::HeaderName, header::HeaderValue, HeaderMap};
use ntex::rt;
use ntex::service::{fn_factory_with_config, fn_service, Service};
use ntex::web::{self, ws, Error, HttpRequest, HttpResponse};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, trace, warn};

use hive_router_plan_executor::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};

use crate::jwt::errors::JwtError;
use crate::pipeline::coerce_variables::coerce_request_variables;
use crate::pipeline::error::PipelineError;
use crate::pipeline::execute_pipeline;
use crate::pipeline::execution::{ExposeQueryPlanMode, EXPOSE_QUERY_PLAN_HEADER};
use crate::pipeline::execution_request::ExecutionRequest;
use crate::pipeline::introspection_policy::handle_introspection_policy;
use crate::pipeline::normalize::normalize_request_with_cache;
use crate::pipeline::parser::parse_operation_with_cache;
use crate::pipeline::validation::validate_operation_with_cache;
use crate::schema_state::SchemaState;
use crate::shared_state::RouterSharedState;

type WsStateRef = Rc<RefCell<WsState<tokio::sync::mpsc::Sender<()>>>>;

pub async fn ws_index(
    req: HttpRequest,
    schema_state: web::types::State<Arc<SchemaState>>,
    shared_state: web::types::State<Arc<RouterSharedState>>,
) -> Result<HttpResponse, Error> {
    let schema_state = schema_state.get_ref().clone();
    let shared_state = shared_state.get_ref().clone();

    let accepted_subprotocol = ws::subprotocols(&req)
        .find(|p| *p == WS_SUBPROTOCOL)
        .map(|_| WS_SUBPROTOCOL);

    ws::start(
        req,
        accepted_subprotocol,
        fn_factory_with_config(move |sink: ws::WsSink| {
            let schema_state = schema_state.clone();
            let shared_state = shared_state.clone();
            async move {
                ws_service(
                    accepted_subprotocol.is_some(),
                    sink,
                    schema_state,
                    shared_state,
                )
                .await
            }
        }),
    )
    .await
}

async fn ws_service(
    has_accepted_subprotocol: bool,
    sink: ws::WsSink,
    schema_state: Arc<SchemaState>,
    shared_state: Arc<RouterSharedState>,
) -> Result<impl Service<ws::Frame, Response = Option<ws::Message>, Error = io::Error>, web::Error>
{
    // stop lingering keep-alive timer from the H1 dispatcher that upgraded
    // this connection. basically, the H1 dispatcher timer is set on the same IO object
    // and will fire into the WS dispatcher, causing it to terminate with a http keep-alive
    // error (close code 1006).
    //
    // TODO: fix in ntex instead
    sink.io().stop_timer();

    if !has_accepted_subprotocol {
        debug!("WebSocket connection rejecting due to unacceptable subprotocol");
        let _ = sink.send(CloseCode::SubprotocolNotAcceptable.into()).await;
        // we dont return an Err here because we want to gracefully close the
        // connection for the client side with a close frame. returning an Err
        // would result in an abrupt termination of the connection
    } else {
        debug!("WebSocket connection accepted");
    }

    let (heartbeat_tx, heartbeat_rx) = oneshot::channel();
    let (acknowledged_tx, acknowledged_rx) = oneshot::channel();

    let state: WsStateRef = Rc::new(RefCell::new(WsState::new(acknowledged_tx)));

    rt::spawn(heartbeat(state.clone(), sink.clone(), heartbeat_rx));
    rt::spawn(handshake_timeout(
        state.clone(),
        sink.clone(),
        acknowledged_rx,
        CloseCode::ConnectionInitTimeout,
    ));

    let heartbeat_tx = Rc::new(RefCell::new(Some(heartbeat_tx)));
    let state_for_service = state.clone();
    let service = fn_service(move |frame| {
        let sink = sink.clone();
        let state = state_for_service.clone();
        let schema_state = schema_state.clone();
        let shared_state = shared_state.clone();
        let heartbeat_tx = heartbeat_tx.clone();
        async move {
            match parse_frame_to_text(frame, &state) {
                Ok(text) => {
                    Ok(handle_text_frame(text, sink, state, &schema_state, &shared_state).await)
                }
                Err(FrameNotParsedToText::Message(msg)) => Ok(Some(msg)),
                Err(FrameNotParsedToText::Closed) => {
                    // stop heartbeat and handshake timeout tasks on shutdown
                    if let Some(tx) = heartbeat_tx.borrow_mut().take() {
                        let _ = tx.send(());
                    }
                    if let Some(tx) = state.borrow_mut().acknowledged_tx.take() {
                        let _ = tx.send(());
                    }
                    // clearing the map will drop all the senders, which will
                    // in turn cancel all active subscription streams and perform
                    // the cleanup in there
                    state.borrow_mut().subscriptions.clear();
                    // we dont need to emit anything here because the conneciton is already closed
                    Ok(None)
                }
                Err(FrameNotParsedToText::None) => Ok(None),
            }
        }
    });

    Ok(service)
}

/// Ensure a subscription is removed from active subscriptions when dropped (server-side).
struct SubscriptionGuard {
    state: WsStateRef,
    id: String,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        self.state.borrow_mut().subscriptions.remove(&self.id);
        trace!(id = %self.id, "Subscription removed from active subscriptions");
    }
}

async fn handle_text_frame(
    text: String,
    sink: ws::WsSink,
    state: WsStateRef,
    schema_state: &Arc<SchemaState>,
    shared_state: &Arc<RouterSharedState>,
) -> Option<ws::Message> {
    let client_msg: ClientMessage = match sonic_rs::from_str(&text) {
        Ok(msg) => msg,
        Err(e) => {
            error!("Failed to parse client message to JSON: {}", e);
            return Some(CloseCode::BadRequest("Invalid message received from client").into());
        }
    };

    trace!("type" = client_msg.as_ref(), "Received client message");

    match client_msg {
        ClientMessage::ConnectionInit { payload } => {
            if state.borrow().handshake_received {
                return Some(CloseCode::TooManyInitialisationRequests.into());
            }
            state.borrow_mut().handshake_received = true;
            state.borrow_mut().init_payload = payload;
            state.borrow_mut().complete_handshake();

            let _ = sink.send(ServerMessage::ack()).await;

            debug!("Connection acknowledged");

            let header_map =
                parse_headers_from_connection_init_payload(state.borrow().init_payload.as_ref());
            if !header_map.is_empty() {
                trace!("Connection init message contains headers in the payload");
            } else {
                trace!("Connection init message does not contain headers in the payload");
            }

            None
        }
        ClientMessage::Subscribe { id, payload } => {
            if let Some(msg) = state.borrow().check_acknowledged() {
                return Some(msg);
            }

            if state.borrow().subscriptions.contains_key(&id) {
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
                        &[GraphQLError::from_message_and_extensions(
                            "No supergraph available yet".to_string(),
                            GraphQLErrorExtensions::new_from_code("SERVICE_UNAVAILABLE"),
                        )],
                    ));
                }
            };

            let config = &shared_state.router_config.websocket;

            let connection_init_headers = if config.headers.accepts_connection_headers() {
                let state_borrow = state.borrow();
                parse_headers_from_connection_init_payload(state_borrow.init_payload.as_ref())
            } else {
                HeaderMap::new()
            };

            let extensions_headers = if config.headers.accepts_operation_headers() {
                parse_headers_from_extensions(payload.extensions.as_ref())
            } else {
                HeaderMap::new()
            };

            // merge, extensions have precedence
            let mut headers = connection_init_headers;
            for (key, value) in extensions_headers.iter() {
                headers.insert(key.clone(), value.clone());
            }

            // store the merged headers back to init_payload if configured to do so
            if config.headers.persist {
                if let Some(ref mut init_payload) = state.borrow_mut().init_payload {
                    for (key, value) in headers.iter() {
                        if let Ok(val_str) = value.to_str() {
                            init_payload
                                .fields
                                .insert(key.to_string(), Value::from(val_str));
                        }
                    }
                }
            }

            let mut payload = ExecutionRequest {
                query: payload.query,
                operation_name: payload.operation_name,
                variables: payload.variables.unwrap_or_default(),
                extensions: payload.extensions,
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

            let jwt_request_details = match &shared_state.jwt_auth_runtime {
                Some(jwt_auth_runtime) => match jwt_auth_runtime
                    .validate_headers(&headers, &shared_state.jwt_claims_cache)
                    .await
                {
                    Ok(Some(jwt_context)) => JwtRequestDetails::Authenticated {
                        scopes: jwt_context.extract_scopes(),
                        claims: match jwt_context
                            .get_claims_value()
                            .map_err(PipelineError::JwtForwardingError)
                        {
                            Ok(claims) => claims,
                            Err(e) => return Some(e.into_server_message(&id)),
                        },
                        token: jwt_context.token_raw,
                        prefix: jwt_context.token_prefix,
                    },
                    Ok(None) => JwtRequestDetails::Unauthenticated,
                    // jwt_auth_runtime.validate_headers() will error out only if
                    // authentication is required and has failed. we therefore use
                    // the JwtError conversion here to respond with proper error message
                    // close with Forbidden.
                    Err(e) => {
                        let _ = sink.send(e.clone().into_server_message(&id)).await;
                        // we report error as graphql error, but we also close the
                        // connection since we're dealing with auth so let's be safe
                        return Some(e.into_close_message());
                    }
                },
                None => JwtRequestDetails::Unauthenticated,
            };

            // synthetic client request details for plan executor
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

            if normalize_payload.operation_for_introspection.is_some() {
                if let Err(e) = handle_introspection_policy(
                    &shared_state.introspection_policy,
                    &client_request_details,
                ) {
                    return Some(e.into_server_message(&id));
                }
            }

            let variable_payload = match coerce_request_variables(
                supergraph,
                &mut payload.variables,
                &normalize_payload,
            ) {
                Ok(payload) => payload,
                Err(err) => return Some(err.into_server_message(&id)),
            };

            let query_plan_cancellation_token =
                CancellationToken::with_timeout(shared_state.router_config.query_planner.timeout);

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
                        .subscriptions
                        .insert(id.clone(), cancel_tx);

                    // automatically remove the subscription from subscriptions when dropped
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
                        // we dont emit complete on cancelled subscriptions.
                        // they're either deliberately cancelled by the client
                        // or dropped due to connection close, either way
                        // we dont/cant inform the client with a complete message
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
            if let Some(msg) = state.borrow().check_acknowledged() {
                return Some(msg);
            }

            if let Some(cancel_tx) = state.borrow_mut().subscriptions.remove(&id) {
                trace!(id = %id, "Client requested subscription cancellation");
                let _ = cancel_tx.try_send(());
            }
            None
        }
        ClientMessage::Ping {} => {
            // respond with pong always, regardless of acknowledged state
            // the client should be able to use subprotocol pings/pongs to check liveness
            Some(ServerMessage::pong())
        }
        ClientMessage::Pong {} => None,
    }
}

/// Parses headers from a sonic_rs::Object into a HeaderMap. Only stringifiable
/// values are included; nulls, arrays, objects and other non-primitive values
/// are ignored.
fn parse_headers_from_object(headers_obj: &sonic_rs::Object) -> HeaderMap {
    let mut header_map = HeaderMap::new();

    for (key, value) in headers_obj.iter() {
        let value_str = if let Some(s) = value.as_str() {
            Some(s.to_string())
        } else if let Some(b) = value.as_bool() {
            Some(b.to_string())
        } else if value.is_number() {
            Some(value.to_string())
        } else {
            None // ignore nulls, arrays, objects
        };

        if let Some(val_str) = value_str {
            let key_str: &str = key;
            if let (Ok(name), Ok(val)) = (
                HeaderName::try_from(key_str),
                HeaderValue::try_from(val_str),
            ) {
                header_map.insert(name, val);
            }
        }
    }

    header_map
}

fn parse_headers_from_connection_init_payload(
    payload: Option<&ConnectionInitPayload>,
) -> HeaderMap {
    let mut header_map = HeaderMap::new();
    if let Some(payload) = payload {
        // First check if there's a nested "headers" object
        if let Some(headers_prop) = payload.fields.get("headers") {
            if let Some(headers_obj) = headers_prop.as_object() {
                header_map = parse_headers_from_object(headers_obj);
                return header_map;
            }
        }

        // If no nested "headers" object, treat all top-level fields as potential headers
        // Convert the entire fields HashMap to a sonic_rs::Object
        let mut obj = sonic_rs::Object::new();
        for (k, v) in payload.fields.iter() {
            obj.insert(k, v.clone());
        }
        header_map = parse_headers_from_object(&obj);
    }
    header_map
}

fn parse_headers_from_extensions(extensions: Option<&HashMap<String, Value>>) -> HeaderMap {
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

// NOTE: no `From` trait because it can into ws message and ws closecode but both are ws::Message
impl PipelineError {
    fn into_server_message(self, id: &str) -> ws::Message {
        let code = self.graphql_error_code();
        let message = self.graphql_error_message();

        let graphql_error = GraphQLError::from_message_and_extensions(
            message,
            GraphQLErrorExtensions::new_from_code(code),
        );

        ServerMessage::error(id, &[graphql_error])
    }
}

// NOTE: no `From` trait because it can into ws message and ws closecode but both are ws::Message
impl JwtError {
    fn into_server_message(self, id: &str) -> ws::Message {
        ServerMessage::error(
            id,
            &[GraphQLError::from_message_and_code(
                self.to_string(),
                self.error_code(),
            )],
        )
    }
    fn into_close_message(self) -> ws::Message {
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

        let headers = parse_headers_from_object(&headers_obj);

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

    // TODO: hella tests
}
