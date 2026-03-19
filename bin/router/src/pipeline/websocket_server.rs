use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use hive_router_internal::telemetry::traces::spans::graphql::GraphQLOperationSpan;
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
use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use hive_router_plan_executor::plugin_context::{
    PluginContext, PluginRequestState, RouterHttpRequest,
};
use hive_router_plan_executor::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};
use hive_router_query_planner::state::supergraph_state::OperationKind;
use http::Method;
use ntex::channel::oneshot;
use ntex::http::{header::HeaderName, header::HeaderValue, HeaderMap};
use ntex::router::Path;
use ntex::service::{fn_factory_with_config, fn_service, fn_shutdown, Service};
use ntex::web::{self, ws, Error, HttpRequest, HttpResponse};
use ntex::{chain, rt};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use tokio::sync::mpsc;
use tracing::{debug, error, trace, warn, Instrument};

use crate::jwt::errors::JwtError;
use crate::pipeline::coerce_variables::coerce_request_variables;
use crate::pipeline::error::PipelineError;
use crate::pipeline::execute_pipeline;
use crate::pipeline::execution_request::GetQueryStr;
use crate::pipeline::normalize::normalize_request_with_cache;
use crate::pipeline::parser::parse_operation_with_cache;
use crate::pipeline::usage_reporting;
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

    let plugin_context = req.extensions().get::<Arc<PluginContext>>().cloned();

    ws::start(
        req,
        accepted_subprotocol,
        fn_factory_with_config(move |sink: ws::WsSink| {
            let schema_state = schema_state.clone();
            let shared_state = shared_state.clone();
            let plugin_context = plugin_context.clone();
            async move {
                ws_service(
                    accepted_subprotocol.is_some(),
                    sink,
                    schema_state,
                    shared_state,
                    plugin_context,
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
    plugin_context: Option<Arc<PluginContext>>,
) -> Result<impl Service<ws::Frame, Response = Option<ws::Message>, Error = io::Error>, web::Error>
{
    if !has_accepted_subprotocol {
        debug!("WebSocket connection rejecting due to unacceptable subprotocol");
        let _ = sink.send(CloseCode::SubprotocolNotAcceptable.into()).await;
        // we dont return an Err here because we want to gracefully close the
        // connection for the client side with a close frame. returning an Err
        // would result in an abrupt termination of the connection
    } else {
        debug!("WebSocket connection accepted");
    }

    let ws_uri: Rc<http::Uri> = Rc::new(
        shared_state
            .router_config
            .websocket_path()
            .expect("websocket path must exist because the websocket handler wouldn't have been mounted otherwise")
            .parse()
            .unwrap_or_else(|_| http::Uri::from_static("/graphql")),
    );
    let ws_path: Rc<Path<http::Uri>> = Rc::new(Path::new((*ws_uri).clone()));

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

    let state_for_service = state.clone();
    let service = fn_service(move |frame| {
        let sink = sink.clone();
        let state = state_for_service.clone();
        let schema_state = schema_state.clone();
        let shared_state = shared_state.clone();
        let plugin_context = plugin_context.clone();
        let ws_uri = ws_uri.clone();
        let ws_path = ws_path.clone();
        async move {
            match parse_frame_to_text(frame, &state) {
                Ok(text) => Ok(handle_text_frame(
                    text,
                    sink,
                    state,
                    &schema_state,
                    &shared_state,
                    plugin_context,
                    &ws_uri,
                    &ws_path,
                )
                .await),
                Err(FrameNotParsedToText::Message(msg)) => Ok(Some(msg)),
                Err(FrameNotParsedToText::Closed) => {
                    // we dont need to emit anything here because the conneciton is already closed
                    Ok(None)
                }
                Err(FrameNotParsedToText::None) => Ok(None),
            }
        }
    });

    let on_shutdown = fn_shutdown(async move || {
        // stop heartbeat and handshake timeout tasks on shutdown
        let _ = heartbeat_tx.send(());
        if let Some(tx) = state.borrow_mut().acknowledged_tx.take() {
            let _ = tx.send(());
        }
        // clearing the map will drop all the senders, which will
        // in turn cancel all active subscription streams and perform
        // the cleanup in there
        state.borrow_mut().subscriptions.clear();
    });

    Ok(chain(service).and_then(on_shutdown))
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
    plugin_context: Option<Arc<PluginContext>>,
    ws_uri: &http::Uri,
    ws_path: &Path<http::Uri>,
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

            let started_at = Instant::now();
            let operation_span = GraphQLOperationSpan::new();

            let result = async {
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

                let mut payload = GraphQLParams {
                    query: Some(payload.query),
                    operation_name: payload.operation_name,
                    variables: payload.variables.unwrap_or_default(),
                    extensions: payload.extensions,
                };

                // synthetic router http request for plugins - there's no real http request
                // in the ws subscribe flow, so we assemble one from the ws path and merged headers.
                // of course there is the http upgrade request, but that one is useless for the plugin system
                let plugin_req_state = if let (Some(plugins), Some(ref plugin_context)) = (
                    shared_state.plugins.as_ref(),
                    plugin_context,
                ) {
                    Some(PluginRequestState {
                        plugins: plugins.clone(),
                        router_http_request: RouterHttpRequest {
                            uri: &ws_uri,
                            method: &Method::POST,
                            version: http::Version::HTTP_11,
                            headers: &headers,
                            path: ws_uri.path(),
                            query_string: ws_uri.query().unwrap_or(""),
                            match_info: &ws_path,
                        },
                        context: plugin_context.clone(),
                    })
                } else {
                    None
                };

                let client_name = headers
                    .get(
                        &shared_state
                            .router_config
                            .telemetry
                            .client_identification
                            .name_header,
                    )
                    .and_then(|v| v.to_str().ok());
                let client_version = headers
                    .get(
                        &shared_state
                            .router_config
                            .telemetry
                            .client_identification
                            .version_header,
                    )
                    .and_then(|v| v.to_str().ok());

                let parser_result =
                    match parse_operation_with_cache(shared_state, &payload, &plugin_req_state).await {
                        Ok(result) => result,
                        Err(err) => return Some(err.into_server_message(&id)),
                    };

                let parser_payload = match parser_result {
                    crate::pipeline::parser::ParseResult::Payload(payload) => payload,
                    crate::pipeline::parser::ParseResult::EarlyResponse(_) => {
                        return Some(ServerMessage::error(
                            &id,
                            &[GraphQLError::from_message_and_code(
                                "Unexpected early response during parse",
                                "INTERNAL_SERVER_ERROR",
                            )],
                        ));
                    }
                };

                operation_span.record_details(
                    &parser_payload.minified_document,
                    (&parser_payload).into(),
                    client_name,
                    client_version,
                    &parser_payload.hive_operation_hash,
                );

                match validate_operation_with_cache(
                    supergraph,
                    schema_state,
                    shared_state,
                    &parser_payload,
                    &plugin_req_state,
                )
                .await
                {
                    Ok(Some(_)) => {
                        return Some(ServerMessage::error(
                            &id,
                            &[GraphQLError::from_message_and_code(
                                "Unexpected early response during validation",
                                "INTERNAL_SERVER_ERROR",
                            )],
                        ));
                    }
                    Ok(None) => {}
                    Err(err) => return Some(err.into_server_message(&id)),
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

                let is_subscription = matches!(
                    normalize_payload.operation_for_plan.operation_kind,
                    Some(OperationKind::Subscription)
                );

                if is_subscription && !shared_state.router_config.subscriptions.enabled {
                    return Some(PipelineError::SubscriptionsNotSupported.into_server_message(&id));
                }

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

                let variable_payload = match coerce_request_variables(
                    supergraph,
                    &mut payload.variables,
                    &normalize_payload,
                ) {
                    Ok(payload) => payload,
                    Err(err) => return Some(err.into_server_message(&id)),
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
                        query: match payload.get_query() {
                            Ok(q) => q,
                            Err(e) => return Some(e.into_server_message(&id)),
                        },
                    },
                    jwt: jwt_request_details,
                };

                match execute_pipeline(
                    &client_request_details,
                    &normalize_payload,
                    &variable_payload,
                    supergraph,
                    shared_state,
                    schema_state,
                    &operation_span,
                    &plugin_req_state,
                )
                .await
                {
                    Ok(QueryPlanExecutionResult::Single(response)) => {
                        if let Some(hive_usage_agent) = &shared_state.hive_usage_agent {
                            usage_reporting::collect_usage_report(
                                supergraph.supergraph_schema.clone(),
                                started_at.elapsed(),
                                client_name,
                                client_version,
                                &client_request_details,
                                hive_usage_agent,
                                shared_state
                                    .router_config
                                    .telemetry
                                    .hive
                                    .as_ref()
                                    .map(|c| &c.usage_reporting)
                                    .expect(
                                        // SAFETY: According to `configure_app_from_config` in `bin/router/src/lib.rs`,
                                        // the UsageAgent is only created when usage reporting is enabled.
                                        // Thus, this expect should never panic.
                                        "Expected Usage Reporting options to be present when Hive Usage Agent is initialized",
                                    ),
                                response.error_count,
                            )
                            .await;
                        }

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
            .instrument(operation_span.clone())
            .await;

            result
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
