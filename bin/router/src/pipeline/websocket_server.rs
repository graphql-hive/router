use crate::executor::headers::response::ResponseHeaderSink;
use futures::StreamExt;
use hive_console_sdk::agent::usage_agent::RequestDetails;
use http::Method;
use ntex::channel::oneshot;
use ntex::http::{header::HeaderName, header::HeaderValue, HeaderMap};
use ntex::router::Path;
use ntex::rt;
use ntex::service::{Ctx, Service};
use ntex::url::Url;
use ntex::web::{ws, HttpRequest, HttpResponse};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, error, trace, warn, Instrument};

use crate::executor::executors::graphql_transport_ws::{
    ClientMessage, CloseCode, ConnectionInitPayload, ServerMessage, WS_SUBPROTOCOL,
};
use crate::executor::executors::websocket_common::{
    handshake_timeout, heartbeat, parse_frame_to_text, FrameNotParsedToText, WsState,
};
use crate::executor::hooks::on_graphql_params::GraphQLParams;
use crate::executor::plugin_context::{PluginContext, PluginRequestState, RouterHttpRequest};
use crate::executor::request_context::{RequestContextExt, SharedRequestContext};
use crate::executor::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};
use crate::telemetry::metrics::catalog::values::SubscriptionEndReason;
use crate::telemetry::metrics::subscription_metrics::{
    ActiveClientConnectionGuard, SubscriptionTransport,
};
use crate::telemetry::traces::spans::graphql::GraphQLOperationSpan;

use crate::jwt::errors::JwtError;
use crate::pipeline::active_subscriptions::SubscriptionEvent;
use crate::pipeline::error::{ClientPipelineError, PipelineError};
use crate::pipeline::execution_request::{OperationPreparation, OperationPreparationResult};
use crate::pipeline::header::{ResponseMode, SingleContentType, StreamContentType};
use crate::pipeline::operation::{
    execute_operation, OperationOutcome, OperationRequest, Transport,
};
use crate::schema_state::SchemaState;
use crate::shared_state::{RouterSharedState, SharedRouterResponse};
use crate::telemetry::logging::request_id::WithRequestIdentifiers;
use crate::telemetry::logging::summary::{self, WithRequestSummary};
use crate::telemetry::logging::targets;
use crate::telemetry::request_scope::RequestTaskScope;
use crate::telemetry::{traces::hive_trace_context::HiveTraceScope, HeaderExtractor};
use crate::RouterAppState;

type WsStateRef = Rc<RefCell<WsState<tokio::sync::mpsc::Sender<()>>>>;

pub async fn ws_index(state: &RouterAppState, _: (), req: HttpRequest) -> HttpResponse {
    // the ws connection service outlives the handler, so it owns its own arcs
    let schema_state = state.schema.clone();
    let shared_state = state.shared.clone();

    let accepted_subprotocol = ws::subprotocols(&req)
        .find(|p| *p == WS_SUBPROTOCOL)
        .map(|_| WS_SUBPROTOCOL);

    let plugin_context = req.extensions().get::<Arc<PluginContext>>().cloned();
    let request_context = match req.read_request_context() {
        Ok(ctx) => ctx,
        Err(_) => return HttpResponse::BadRequest().build(),
    };

    let service = WsConnectionService::new(
        accepted_subprotocol.is_some(),
        schema_state,
        shared_state,
        req.clone(),
        plugin_context,
        request_context,
    );

    // drives the whole connection, it resolves only once the connection is closed
    match ws::start(&req, accepted_subprotocol, service).await {
        // a rejected handshake gets its proper error status. when ntex still had the io it
        // already wrote this response itself, otherwise (like a missing `Connection: upgrade`)
        // this is the response the client gets
        Err(ntex::ws::error::WsError::Handshake(err)) => {
            debug!(target: targets::WEBSOCKET_SERVER, error = ?err, "websocket handshake rejected");
            ntex::http::ResponseError::error_response(&err)
        }
        result => {
            if let Err(err) = result {
                debug!(target: targets::WEBSOCKET_SERVER, error = ?err, "websocket connection terminated with an error");
            }
            // the upgraded connection belongs to the websocket dispatcher, so this response is
            // never sent. it carries the 101 that went out on the wire, so request summaries
            // and plugin `on_end` callbacks see the real status
            HttpResponse::new(ntex::http::StatusCode::SWITCHING_PROTOCOLS)
        }
    }
}

/// Serves the frames of a single GraphQL over WebSocket connection.
///
/// ntex hands the connection's sink to the service as its state on every call. The tasks
/// that need the sink from the start of the connection (the heartbeat and the handshake
/// timeout) are therefore started on the first readiness check, which the websocket
/// dispatcher performs before reading the first frame.
struct WsConnectionService {
    has_accepted_subprotocol: bool,
    state: WsStateRef,
    schema_state: Arc<SchemaState>,
    shared_state: Arc<RouterSharedState>,
    req: HttpRequest,
    plugin_context: Option<Arc<PluginContext>>,
    request_context: SharedRequestContext,
    ws_uri: Url,
    ws_path: Path<Url>,
    /// Headers listed in `websocket.headers.upgrade`, taken from the upgrade request.
    upgrade_headers: HeaderMap,
    /// Stop receivers for the heartbeat and handshake timeout tasks, taken when the tasks start.
    pending_tasks: RefCell<Option<(oneshot::Receiver<()>, oneshot::Receiver<()>)>>,
    heartbeat_tx: RefCell<Option<oneshot::Sender<()>>>,
    conn_guard: RefCell<Option<ActiveClientConnectionGuard>>,
}

impl WsConnectionService {
    fn new(
        has_accepted_subprotocol: bool,
        schema_state: Arc<SchemaState>,
        shared_state: Arc<RouterSharedState>,
        req: HttpRequest,
        plugin_context: Option<Arc<PluginContext>>,
        request_context: SharedRequestContext,
    ) -> Self {
        let conn_guard: Option<ActiveClientConnectionGuard> = if has_accepted_subprotocol {
            Some(
                shared_state
                    .telemetry_context
                    .metrics
                    .subscriptions
                    .active_client_connection(SubscriptionTransport::WebSocket),
            )
        } else {
            None
        };

        let ws_uri: Url = shared_state
            .router_config
            .websocket_path()
            .expect("websocket path must exist because the websocket handler wouldn't have been mounted otherwise")
            .parse()
            .unwrap_or_else(|_| Url::from_static("/graphql"));
        let ws_path = Path::new(ws_uri.clone());

        let mut upgrade_headers = HeaderMap::new();
        for name in &shared_state.router_config.websocket.headers.upgrade {
            let name = name.get_header_ref();
            for value in req.headers().get_all(name) {
                upgrade_headers.append(name.clone(), value.clone());
            }
        }

        let (heartbeat_tx, heartbeat_rx) = oneshot::channel();
        let (acknowledged_tx, acknowledged_rx) = oneshot::channel();

        Self {
            has_accepted_subprotocol,
            state: Rc::new(RefCell::new(WsState::new(acknowledged_tx))),
            schema_state,
            shared_state,
            req,
            plugin_context,
            request_context,
            ws_uri,
            ws_path,
            upgrade_headers,
            pending_tasks: RefCell::new(Some((heartbeat_rx, acknowledged_rx))),
            heartbeat_tx: RefCell::new(Some(heartbeat_tx)),
            conn_guard: RefCell::new(conn_guard),
        }
    }
}

impl Service<ws::WsSink, ws::Frame> for WsConnectionService {
    type Res = Option<ws::Message>;
    type Error = io::Error;

    async fn ready(&self, ctx: Ctx<'_, Self, ws::WsSink>) -> Result<(), Self::Error> {
        // only the first check starts the tasks, later checks find nothing pending
        let pending_tasks = self.pending_tasks.borrow_mut().take();
        if let Some((heartbeat_rx, acknowledged_rx)) = pending_tasks {
            let sink = ctx.st().clone();

            if !self.has_accepted_subprotocol {
                warn!(target: targets::WEBSOCKET_SERVER, "websocket connection rejecting due to unacceptable subprotocol");
                let _ = sink.send(CloseCode::SubprotocolNotAcceptable.into()).await;
                // we dont return an Err here because we want to gracefully close the
                // connection for the client side with a close frame. returning an Err
                // would result in an abrupt termination of the connection
            } else {
                debug!(target: targets::WEBSOCKET_SERVER, "websocket connection accepted");
            }

            rt::spawn(heartbeat(self.state.clone(), sink.clone(), heartbeat_rx));
            rt::spawn(handshake_timeout(
                self.state.clone(),
                sink,
                acknowledged_rx,
                CloseCode::ConnectionInitTimeout,
            ));
        }
        Ok(())
    }

    async fn call(
        &self,
        frame: ws::Frame,
        ctx: Ctx<'_, Self, ws::WsSink>,
    ) -> Result<Self::Res, Self::Error> {
        match parse_frame_to_text(frame, &self.state) {
            Ok(text) => Ok(handle_text_frame(
                text,
                ctx.st().clone(),
                self.state.clone(),
                &self.schema_state,
                &self.shared_state,
                &self.req,
                self.plugin_context.clone(),
                &self.request_context,
                &self.ws_uri,
                &self.ws_path,
                &self.upgrade_headers,
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

    async fn shutdown(&self, _: Ctx<'_, Self, ws::WsSink>) {
        // stop heartbeat and handshake timeout tasks on shutdown
        if let Some(tx) = self.heartbeat_tx.borrow_mut().take() {
            let _ = tx.send(());
        }
        if let Some(tx) = self.state.borrow_mut().acknowledged_tx.take() {
            let _ = tx.send(());
        }
        // clearing the map will drop all the senders, which will
        // in turn cancel all active subscription streams and perform
        // the cleanup in there
        self.state.borrow_mut().subscriptions.clear();
        // drop conn_guard here to decrement the connection counter
        self.conn_guard.borrow_mut().take();
    }
}

/// Ensure a subscription is removed from active subscriptions when dropped (server-side).
struct SubscriptionGuard {
    state: WsStateRef,
    id: String,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        self.state.borrow_mut().subscriptions.remove(&self.id);
        trace!(target: targets::WEBSOCKET_SERVER, id = %self.id, "Subscription removed from active subscriptions");
    }
}

lazy_static::lazy_static! {
    static ref WS_URI_PATH: http::Uri = http::Uri::from_static("/graphql");
}

#[allow(clippy::too_many_arguments)]
async fn handle_text_frame(
    text: String,
    sink: ws::WsSink,
    state: WsStateRef,
    schema_state: &Arc<SchemaState>,
    shared_state: &Arc<RouterSharedState>,
    req: &HttpRequest,
    plugin_context: Option<Arc<PluginContext>>,
    request_context: &SharedRequestContext,
    ws_uri: &Url,
    ws_path: &Path<Url>,
    upgrade_headers: &HeaderMap,
) -> Option<ws::Message> {
    // IMPORTANT: we intentionally check if a supergraph is selected this "late" because
    // browsers cannot interpret a rejected HTTP Upgrade request and will show a cryptic
    // error to clients. instead, we accept the connection but close it immediately if no
    // supergraph is available, which results in a more understandable error
    let supergraph = match schema_state.select_supergraph(req).await {
        Ok(supergraph) => supergraph,
        Err(err) => {
            error!(target: targets::WEBSOCKET_SERVER, err = ?err, "Supergraph runtime error");
            return Some(
                CloseCode::InternalServerError(Some("Supergraph runtime error".to_string())).into(),
            );
        }
    };
    let supergraph = supergraph.as_ref();

    // TODO: cover response header aggregation for WS
    let response_header_sink = ResponseHeaderSink::default();
    let client_msg: ClientMessage = match sonic_rs::from_str(&text) {
        Ok(msg) => msg,
        Err(e) => {
            error!(target: targets::WEBSOCKET_SERVER, error = ?e, "Failed to parse client message to JSON");
            return Some(CloseCode::BadRequest("Invalid message received from client").into());
        }
    };

    trace!(target: targets::WEBSOCKET_SERVER, type = client_msg.as_ref(), "Received client message");

    match client_msg {
        ClientMessage::ConnectionInit { payload } => {
            if state.borrow().handshake_received {
                warn!(target: targets::WEBSOCKET_SERVER, "Received multiple connection init messages, rejecting connection");

                return Some(CloseCode::TooManyInitialisationRequests.into());
            }
            state.borrow_mut().handshake_received = true;
            state.borrow_mut().init_payload = payload;
            state.borrow_mut().complete_handshake();

            let _ = sink.send(ServerMessage::ack()).await;

            debug!(target: targets::WEBSOCKET_SERVER, "websocket connection acknowledged");

            let header_map =
                parse_headers_from_connection_init_payload(state.borrow().init_payload.as_ref());
            if !header_map.is_empty() {
                trace!(target: targets::WEBSOCKET_SERVER, "Connection init message contains headers in the payload");
            } else {
                trace!(target: targets::WEBSOCKET_SERVER, "Connection init message does not contain headers in the payload");
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
            if let Some(selected) = supergraph {
                operation_span.record_supergraph_name(&selected.snapshot.name);
                operation_span.record_hive_target(selected.snapshot.options.hive_target.as_deref());
            }
            let span_clone = operation_span.clone();
            let hive_trace_scope = HiveTraceScope::new();

            let result = hive_trace_scope.scope(async {
                async {
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

                // done after persisting so that init_payload only ever holds what the client sent
                for name in &config.headers.upgrade {
                    headers.remove(name.get_header_ref());
                }
                for (name, value) in upgrade_headers.iter() {
                    headers.append(name.clone(), value.clone());
                }

                let parent_ctx = shared_state
                    .telemetry_context
                    .extract_context(&HeaderExtractor(&headers));
                let request_identifiers =
                  Arc::new(shared_state
                        .telemetry_context
                        .logging_correlation_extractor
                        .extract(&headers, &parent_ctx));
                let headers = Arc::new(headers);

                let inner_res = async {
                  let _summary = summary::SummaryOnDrop::new(started_at);
                  let Some(supergraph) = supergraph else {
                      // IMPORTANT: we dont do this earlier because router might be loading the
                      // supergraph as we speak, so we simply reject the operation request instead
                      error!(target: targets::WEBSOCKET_SERVER, "No supergraph available yet, unable to process client subscribe message");

                      return Some(ServerMessage::error(
                          &id,
                          &[GraphQLError::from_message_and_extensions(
                              "No supergraph available yet".to_string(),
                              GraphQLErrorExtensions::new_from_code("SERVICE_UNAVAILABLE"),
                          )],
                      ));
                  };

                  if supergraph.snapshot.is_retired() {
                      // the supergraph selected at upgrade time has since been retired (configured
                      // reload, or the owning plugin dropped/replaced its variant) - reject new
                      // operations on this connection rather than create work from a stale schema.
                      return Some(ServerMessage::error(
                          &id,
                          &[GraphQLError::from_message_and_extensions(
                              "Supergraph used by this connection has been retired".to_string(),
                              GraphQLErrorExtensions::new_from_code("SERVICE_UNAVAILABLE"),
                          )],
                      ));
                  }

                  let payload = GraphQLParams {
                      query: (!payload.query.is_empty()).then_some(payload.query),
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
                              uri: ws_uri,
                              method: &Method::POST,
                              version: http::Version::HTTP_11,
                              headers: headers.as_ref(),
                              path: ws_uri.path().as_str(),
                              query_string: ws_uri.query().map(|q| q.as_str()).unwrap_or(""),
                              match_info: ws_path,
                          },
                          context: plugin_context.clone(),
                          request_context: request_context.clone(),
                      })
                  } else {
                      None
                  };

                  let client_name = headers
                      .get(
                          shared_state
                              .router_config
                              .telemetry
                              .client_identification
                              .name_header.get_header_ref(),
                      )
                      .and_then(|v| v.to_str().ok());
                  let client_version = headers
                      .get(
                          shared_state
                              .router_config
                              .telemetry
                              .client_identification
                              .version_header.get_header_ref(),
                      )
                      .and_then(|v| v.to_str().ok());

                  let payload = match OperationPreparation::prepare_websocket(
                      req,
                      shared_state,
                      supergraph,
                      &plugin_req_state,
                      payload,
                      client_name,
                      client_version,
                  )
                  .await
                  {
                      Ok(OperationPreparationResult::Operation(operation)) => {
                          summary::record(|summary| {
                              summary.set_persisted_document_id(
                                  operation.resolved_document_id.as_deref(),
                              )
                          });
                          operation.graphql_params
                      }
                      Ok(OperationPreparationResult::EarlyResponse(response)) => {
                          return forward_early_response(&sink, &id, response).await;
                      }
                      Err(err) => return Some(err.into_server_message(&id, shared_state, &supergraph.snapshot.name)),
                  };

                  // synthetic request details for plan executor
                  let response_mode = ResponseMode::Dual(
                      SingleContentType::default(),
                      StreamContentType::default(),
                  );
                  let method = Method::POST;

                  debug!(
                      target: targets::GRAPHQL_EXECUTION,
                      client_name,
                      client_version,
                      operation_name = payload.operation_name.as_deref(),
                      operation = payload.query.as_deref(),
                      "graphql request started",
                  );

                  summary::record(|s| {
                      s.set_client_info(client_name, client_version);
                      s.set_response_mode(response_mode.as_str());
                  });

                  let usage_request_details = supergraph.runtime.hive_usage_agent.is_some().then(|| {
                      let mut headers_vec = Vec::with_capacity(headers.len());
                      for (name, value) in headers.iter() {
                          if let Ok(val_str) = value.to_str() {
                              headers_vec.push((name.to_string(), val_str.to_string()));
                          }
                      }

                      RequestDetails {
                          method: Method::POST,
                          // the usage agent works with `http::Uri`, an unconvertible path falls back to "/"
                          url: http::Uri::try_from(ws_uri).unwrap_or_default(),
                          headers: headers_vec,
                      }
                  });

                  let outcome = execute_operation(
                      OperationRequest {
                          method: &method,
                          uri: ws_uri,
                          // TODO: WebSocket subscriptions do not yet expose route path params
                          path_params: Default::default(),
                          headers: headers.as_ref().clone(),
                          graphql_params: payload,
                          // The graphql-transport-ws Subscribe payload carries no document id
                          // and this path never invokes the document id resolver.
                          resolved_document_id: None,
                          client_name,
                          client_version,
                          supergraph,
                          plugin_req_state,
                          request_context,
                          response_mode: &response_mode,
                          response_header_sink: response_header_sink.clone(),
                          operation_span,
                          started_at,
                          usage_request_details,
                          transport: Transport::WebSocket,
                      },
                      shared_state,
                      schema_state,
                  )
                  .await;

                  let shared_response = match outcome {
                      Ok(OperationOutcome::Executed { response, .. }) => response,
                      Ok(OperationOutcome::EarlyResponse(response)) => {
                          return forward_early_response(&sink, &id, response).await;
                      }
                      Err(PipelineError::Client(ClientPipelineError::JwtError(err))) => {
                          let _ = sink.send(err.clone().into_server_message(&id, shared_state, &supergraph.snapshot.name)).await;
                          // we report error as graphql error, but we also close the
                          // connection since we're dealing with auth so let's be safe
                          return Some(err.into_close_message());
                      },
                      Err(err) => return Some(err.into_server_message(&id, shared_state, &supergraph.snapshot.name)),
                  };

                  match shared_response {
                      SharedRouterResponse::Single(response) => {
                          let _ = sink.send(ServerMessage::next(&id, &response.body)).await;
                          Some(ServerMessage::complete(&id))
                      }
                      SharedRouterResponse::Stream(response) => {
                          let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);

                          state
                              .borrow_mut()
                              .subscriptions
                              .insert(id.clone(), cancel_tx);

                          let guard = SubscriptionGuard {
                              state: state.clone(),
                              id: id.clone(),
                          };

                          let mut receiver = response
                              .receiver
                              .unwrap_or_else(|| response.body.subscribe());

                          let client_op_guard = shared_state.telemetry_context.metrics.subscriptions.active_client_operation(SubscriptionTransport::WebSocket);
                          let metrics = shared_state.telemetry_context.metrics.clone();

                          trace!(target: targets::WEBSOCKET_SERVER, id = %id, "Subscription started");

                          let sink = sink.clone();
                          let id_for_loop = id.clone();
                          // must be spawned - blocking the frame handler would prevent
                          // ClientMessage::Complete from being received and processed,
                          // making cancellation impossible
                          let request_scope = RequestTaskScope::capture();
                          rt::spawn(request_scope.scope(async move {
                              let _guard = guard;
                              let mut client_op_guard = client_op_guard;
                              let mut cancelled = false;

                              loop {
                                  tokio::select! {
                                      maybe_item = receiver.recv() => {
                                          match maybe_item {
                                              Ok(SubscriptionEvent::Raw(data)) => {
                                                  let _ = sink.send(ServerMessage::next(&id_for_loop, &data)).await;
                                                  metrics.subscriptions.record_client_sent(SubscriptionTransport::WebSocket);
                                              }
                                              Ok(SubscriptionEvent::Error(errors)) => {
                                                  client_op_guard.set_end_reason(SubscriptionEndReason::Error);
                                                  let _ = sink.send(ServerMessage::error(&id_for_loop, &errors)).await;
                                                  break;
                                              }
                                              Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                                  // NOTE: not warn to avoid log spam when receiver starts
                                                  // lagging. users should rely on the lagged_messages
                                                  // metric to detect slow consumers and tune accordingly
                                                  debug!(target: targets::WEBSOCKET_SERVER, id = %id_for_loop, lagged = n, "Broadcast receiver lagged, dropping message");
                                                  metrics.subscriptions.record_client_lag(SubscriptionTransport::WebSocket, n);
                                                  continue;
                                              }
                                              Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                                  client_op_guard.set_end_reason(SubscriptionEndReason::Completed);
                                                  break;
                                              }
                                          }
                                      }
                                      _ = cancel_rx.recv() => {
                                          cancelled = true;
                                          break;
                                      }
                                  }
                              }

                              if cancelled {
                                  trace!(target: targets::WEBSOCKET_SERVER, id = %id_for_loop, "Subscription cancelled");
                              } else {
                                  trace!(target: targets::WEBSOCKET_SERVER, id = %id_for_loop, "Subscription completed");
                                  let _ = sink.send(ServerMessage::complete(&id_for_loop)).await;
                              }
                          }));

                          None
                      }
                  }
                }.with_request_id(request_identifiers.clone()).with_request_summary().await;

                inner_res
            }
            .instrument(span_clone)
            .await
            }).await;

            result
        }
        ClientMessage::Complete { id } => {
            if let Some(msg) = state.borrow().check_acknowledged() {
                return Some(msg);
            }

            if let Some(cancel_tx) = state.borrow_mut().subscriptions.remove(&id) {
                trace!(target: targets::WEBSOCKET_SERVER, id = %id, "Client requested subscription cancellation");
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

/// Sends a response that a plugin or coprocessor ended the operation with as the
/// operation's only result.
async fn forward_early_response(
    sink: &ws::WsSink,
    id: &str,
    mut response: HttpResponse,
) -> Option<ws::Message> {
    let mut response_body = response.take_body();
    let mut body = Vec::new();
    while let Some(chunk) = response_body.next().await {
        match chunk {
            Ok(chunk) => body.extend_from_slice(&chunk),
            Err(err) => {
                error!(target: targets::WEBSOCKET_SERVER, error = ?err, "Failed to read early response body");
                return Some(CloseCode::InternalServerError(None).into());
            }
        }
    }

    // a coprocessor break without a body, or a failed coprocessor stage, ends the operation
    // with a status alone. `next` can't carry that and would close the whole connection, so
    // the status is reported as the operation's error instead
    if sonic_rs::from_slice::<sonic_rs::Value>(&body).is_err() {
        let status = response.status();
        let reason = status.canonical_reason().unwrap_or("Unknown status");
        return Some(ServerMessage::error(
            id,
            &[GraphQLError::from_message_and_code(
                format!("Operation ended with status {} {reason}", status.as_u16()),
                reason.to_uppercase().replace(' ', "_"),
            )],
        ));
    }

    let _ = sink.send(ServerMessage::next(id, &body)).await;
    Some(ServerMessage::complete(id))
}

/// Parses headers from a sonic_rs::Object into a HeaderMap. Only stringifiable
/// values are included; nulls, arrays, objects and other non-primitive values
/// are ignored.
fn parse_headers_from_object(headers_obj: &sonic_rs::Object) -> HeaderMap {
    let mut header_map = HeaderMap::new();

    for (key, value) in headers_obj.iter() {
        let Ok(name) = HeaderName::try_from(key as &str) else {
            continue;
        };
        let header_value = if let Some(s) = value.as_str() {
            HeaderValue::try_from(s).ok()
        } else if let Some(b) = value.as_bool() {
            Some(HeaderValue::from_static(if b { "true" } else { "false" }))
        } else if let Some(n) = value.as_i64() {
            Some(HeaderValue::from(n))
        } else if let Some(n) = value.as_u64() {
            Some(HeaderValue::from(n))
        } else if let Some(f) = value.as_f64() {
            // f64 has no HeaderValue::From impl, string alloc unavoidable
            HeaderValue::try_from(f.to_string()).ok()
        } else {
            None // ignore nulls, arrays, objects
        };
        if let Some(val) = header_value {
            header_map.insert(name, val);
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
    fn into_server_message(
        self,
        id: &str,
        shared_state: &RouterSharedState,
        supergraph_name: &Arc<str>,
    ) -> ws::Message {
        let code = self.graphql_error_code();
        let message = self.graphql_error_message();

        shared_state
            .telemetry_context
            .metrics
            .graphql
            .record_error(code, Some(supergraph_name.clone()));

        let graphql_error = GraphQLError::from_message_and_extensions(
            message,
            GraphQLErrorExtensions::new_from_code(code),
        );

        ServerMessage::error(id, &[graphql_error])
    }
}

// NOTE: no `From` trait because it can into ws message and ws closecode but both are ws::Message
impl JwtError {
    fn into_server_message(
        self,
        id: &str,
        shared_state: &RouterSharedState,
        supergraph_name: &Arc<str>,
    ) -> ws::Message {
        let code = self.error_code();

        shared_state
            .telemetry_context
            .metrics
            .graphql
            .record_error(code, Some(supergraph_name.clone()));

        ServerMessage::error(
            id,
            &[GraphQLError::from_message_and_code(self.to_string(), code)],
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
