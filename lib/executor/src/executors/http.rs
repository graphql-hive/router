use std::sync::Arc;
use std::time::Duration;

use crate::executors::dedupe::unique_leader_fingerprint;
use crate::executors::map::InflightRequestsMap;
use crate::executors::multipart_subscribe;
use crate::executors::sse;
use crate::hooks::on_subgraph_http_request::{
    OnSubgraphHttpRequestHookPayload, OnSubgraphHttpResponseHookPayload,
};
use crate::plugin_context::PluginRequestState;
use crate::plugin_trait::{EndControlFlow, StartControlFlow};
use crate::response::subgraph_response::SubgraphResponse;
use futures::stream::BoxStream;
use hive_router_config::HiveRouterConfig;
use hive_router_internal::telemetry::TelemetryContext;

use async_trait::async_trait;

use bytes::{BufMut, Bytes};
use http::HeaderMap;
use http::HeaderValue;
use http::StatusCode;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Version;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use tokio::sync::Semaphore;
use tracing::{debug, trace};

use crate::executors::common::SubgraphExecutionRequest;
use crate::executors::error::SubgraphExecutorError;
use crate::utils::consts::CLOSE_BRACE;
use crate::utils::consts::COLON;
use crate::utils::consts::COMMA;
use crate::utils::consts::QUOTE;
use crate::{executors::common::SubgraphExecutor, json_writer::write_and_escape_string};
use hive_router_internal::telemetry::traces::spans::http_request::HttpClientRequestSpan;
use hive_router_internal::telemetry::traces::spans::http_request::HttpInflightRequestSpan;
use hive_router_internal::telemetry::Injector;
use http::HeaderName;
use tracing::Instrument;

pub struct HTTPSubgraphExecutor {
    pub subgraph_name: String,
    pub endpoint: http::Uri,
    pub http_client: Arc<HttpClient>,
    pub header_map: HeaderMap,
    pub semaphore: Arc<Semaphore>,
    pub dedupe_enabled: bool,
    pub in_flight_requests: InflightRequestsMap,
    pub telemetry_context: Arc<TelemetryContext>,
    pub config: Arc<HiveRouterConfig>,
}

const FIRST_VARIABLE_STR: &[u8] = b",\"variables\":{";
const FIRST_QUOTE_STR: &[u8] = b"{\"query\":";

pub type HttpClient = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

impl HTTPSubgraphExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        http_client: Arc<HttpClient>,
        semaphore: Arc<Semaphore>,
        dedupe_enabled: bool,
        in_flight_requests: InflightRequestsMap,
        telemetry_context: Arc<TelemetryContext>,
        config: Arc<HiveRouterConfig>,
    ) -> Self {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        header_map.insert(
            http::header::CONNECTION,
            HeaderValue::from_static("keep-alive"),
        );

        Self {
            subgraph_name,
            endpoint,
            http_client,
            header_map,
            semaphore,
            dedupe_enabled,
            in_flight_requests,
            telemetry_context,
            config,
        }
    }

    fn build_request_body<'a>(
        &self,
        execution_request: &SubgraphExecutionRequest<'a>,
    ) -> Result<Vec<u8>, SubgraphExecutorError> {
        let mut body = Vec::with_capacity(4096);
        body.put(FIRST_QUOTE_STR);
        write_and_escape_string(&mut body, execution_request.query);
        let mut first_variable = true;
        if let Some(variables) = &execution_request.variables {
            for (variable_name, variable_value) in variables {
                if first_variable {
                    body.put(FIRST_VARIABLE_STR);
                    first_variable = false;
                } else {
                    body.put(COMMA);
                }
                body.put(QUOTE);
                body.put(variable_name.as_bytes());
                body.put(QUOTE);
                body.put(COLON);
                let value_str = sonic_rs::to_string(variable_value).map_err(|err| {
                    SubgraphExecutorError::VariablesSerializationFailure(
                        variable_name.to_string(),
                        err,
                    )
                })?;
                body.put(value_str.as_bytes());
            }
        }
        if let Some(representations) = &execution_request.representations {
            if first_variable {
                body.put(FIRST_VARIABLE_STR);
                first_variable = false;
            } else {
                body.put(COMMA);
            }
            body.put("\"representations\":".as_bytes());
            body.extend_from_slice(representations);
        }
        // "first_variable" should be still true if there are no variables
        if !first_variable {
            body.put(CLOSE_BRACE);
        }

        if let Some(extensions) = &execution_request.extensions {
            if !extensions.is_empty() {
                let as_value = sonic_rs::to_value(extensions).unwrap();

                body.put(COMMA);
                body.put("\"extensions\":".as_bytes());
                body.extend_from_slice(as_value.to_string().as_bytes());
            }
        }

        body.put(CLOSE_BRACE);

        Ok(body)
    }
}

pub struct SendRequestOpts<'a> {
    pub http_client: &'a HttpClient,
    pub endpoint: &'a http::Uri,
    pub method: http::Method,
    pub body: Vec<u8>,
    pub headers: HeaderMap,
    pub timeout: Option<Duration>,
    pub telemetry_context: Arc<TelemetryContext>,
}

async fn send_request<'a>(
    opts: SendRequestOpts<'a>,
) -> Result<SubgraphHttpResponse, SubgraphExecutorError> {
    let SendRequestOpts {
        http_client,
        endpoint,
        method,
        body,
        headers,
        timeout,
        telemetry_context,
    } = opts;

    let mut req = hyper::Request::builder()
        .method(method)
        .uri(endpoint)
        .version(Version::HTTP_11)
        .body(Full::new(Bytes::from(body)))?;

    *req.headers_mut() = headers;

    debug!("making http request to {}", endpoint.to_string());

    let http_request_span = HttpClientRequestSpan::from_request(&req);

    async {
        // TODO: let's decide at some point if the tracing headers
        //       should be part of the fingerprint or not.
        telemetry_context.inject_context(&mut TraceHeaderInjector(req.headers_mut()));

        let res_fut = http_client.request(req);

        let res = if let Some(timeout_duration) = timeout {
            tokio::time::timeout(timeout_duration, res_fut)
                .await
                .map_err(|_| SubgraphExecutorError::RequestTimeout(timeout_duration.as_millis()))?
        } else {
            res_fut.await
        }?;

        http_request_span.record_response(&res);

        debug!(
            "http request to {} completed, status: {}",
            endpoint.to_string(),
            res.status()
        );

        let (parts, body) = res.into_parts();
        let body = body.collect().await?.to_bytes();

        if body.is_empty() {
            return Err(SubgraphExecutorError::EmptyResponseBody);
        }

        Ok(SubgraphHttpResponse {
            status: parts.status,
            body,
            headers: parts.headers.into(),
        })
    }
    .instrument(http_request_span.clone())
    .await
}

pub enum DeduplicationHint {
    Deduped {
        fingerprint: u64,
        leader_id: u64,
        is_leader: bool,
    },
    NotDeduped,
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    fn endpoint(&self) -> &http::Uri {
        &self.endpoint
    }

    async fn execute<'a>(
        &self,
        mut execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: &'a Option<PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
        let mut body = self.build_request_body(&execution_request)?;

        self.header_map.iter().for_each(|(key, value)| {
            execution_request.headers.insert(key, value.clone());
        });

        let mut method = http::Method::POST;
        let mut deduplicate_request = !self.dedupe_enabled || !execution_request.dedupe;

        let mut on_end_callbacks = vec![];
        let mut response = None;

        if let Some(plugin_req_state) = plugin_req_state.as_ref() {
            let mut start_payload = OnSubgraphHttpRequestHookPayload {
                subgraph_name: &self.subgraph_name,
                endpoint: &self.endpoint,
                method,
                body,
                execution_request,
                deduplicate_request,
                context: &plugin_req_state.context,
            };
            for plugin in plugin_req_state.plugins.as_ref() {
                let result = plugin.on_subgraph_http_request(start_payload).await;
                start_payload = result.payload;
                match result.control_flow {
                    StartControlFlow::Proceed => { /* continue to next plugin */ }
                    StartControlFlow::EndWithResponse(early_response) => {
                        response = Some(early_response);
                        // Break so other plugins are not called
                        break;
                    }
                    StartControlFlow::OnEnd(callback) => {
                        on_end_callbacks.push(callback);
                    }
                }
            }
            // Give the ownership back to variables
            method = start_payload.method;
            execution_request = start_payload.execution_request;
            body = start_payload.body;
            deduplicate_request = start_payload.deduplicate_request;
        }

        let mut deduplication_hint = DeduplicationHint::NotDeduped;

        let mut response = match response {
            Some(resp) => resp,
            None => {
                let send_request_opts = SendRequestOpts {
                    http_client: &self.http_client,
                    endpoint: &self.endpoint,
                    method,
                    body,
                    headers: execution_request.headers,
                    timeout,
                    telemetry_context: self.telemetry_context.clone(),
                };

                if deduplicate_request {
                    // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
                    // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
                    let _permit = self.semaphore.acquire().await.unwrap();
                    send_request(send_request_opts).await?
                } else {
                    let fingerprint = send_request_opts.fingerprint();

                    let inflight_span = HttpInflightRequestSpan::new(
                        &send_request_opts.method,
                        send_request_opts.endpoint,
                        &send_request_opts.headers,
                        &send_request_opts.body,
                    );

                    let result: Result<SubgraphHttpResponse, SubgraphExecutorError> = async {
                        // Clone the cell from the map, dropping the lock from the DashMap immediately.
                        // Prevents any deadlocks.
                        let cell = self
                            .in_flight_requests
                            .entry(fingerprint)
                            .or_default()
                            .clone();
                        // Mark it as a joiner span by default.
                        let mut is_leader = false;
                        let (shared_response, leader_id) = cell
                            .get_or_try_init(|| async {
                                // Override the span to be a leader span for this request.
                                is_leader = true;
                                let res = {
                                    // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
                                    // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
                                    let _permit = self.semaphore.acquire().await.unwrap();
                                    send_request(send_request_opts).await
                                };
                                // It's important to remove the entry from the map before returning the result.
                                // This ensures that once the OnceCell is set, no future requests can join it.
                                // The cache is for the lifetime of the in-flight request only.
                                self.in_flight_requests.remove(&fingerprint);
                                res.map(|res| (res, unique_leader_fingerprint()))
                            })
                            .await?;

                        if is_leader {
                            inflight_span.record_as_leader(leader_id);
                        } else {
                            inflight_span.record_as_joiner(leader_id);
                        }

                        inflight_span
                            .record_response(&shared_response.body, &shared_response.status);

                        deduplication_hint = DeduplicationHint::Deduped {
                            fingerprint,
                            leader_id: *leader_id,
                            is_leader,
                        };

                        Ok(shared_response.clone())
                    }
                    .instrument(inflight_span.clone())
                    .await;

                    result?
                }
            }
        };

        if !on_end_callbacks.is_empty() {
            let mut end_payload = OnSubgraphHttpResponseHookPayload {
                context: &plugin_req_state.as_ref().unwrap().context,
                response,
                deduplication_hint,
            };
            for callback in on_end_callbacks {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Proceed => { /* continue to next plugin */ }
                    EndControlFlow::EndWithResponse(early_response) => {
                        end_payload.response = early_response;
                        // Break so other plugins are not called
                        break;
                    }
                }
            }
            // Give the ownership back to variables
            response = end_payload.response;
        }

        response.deserialize_http_response()
    }

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        connection_timeout: Option<Duration>,
    ) -> Result<BoxStream<'static, SubgraphResponse<'static>>, SubgraphExecutorError> {
        let body = self.build_request_body(&execution_request)?;

        let mut req = hyper::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .version(Version::HTTP_11)
            .body(Full::new(Bytes::from(body)))?;

        let mut headers = execution_request.headers;
        self.header_map.iter().for_each(|(key, value)| {
            headers.insert(key, value.clone());
        });

        // Prefer multipart over SSE for subscriptions
        // https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol
        headers.insert(
            http::header::ACCEPT,
            HeaderValue::from_static(
                r#"multipart/mixed;subscriptionSpec="1.0", text/event-stream"#,
            ),
        );
        *req.headers_mut() = headers;

        debug!(
            "establishing subscription connection to subgraph {} at {}",
            self.subgraph_name,
            self.endpoint.to_string()
        );

        let res_fut = self.http_client.request(req);

        let res = if let Some(timeout_duration) = connection_timeout {
            tokio::time::timeout(timeout_duration, res_fut)
                .await
                .map_err(|_| SubgraphExecutorError::RequestTimeout(timeout_duration.as_millis()))?
        } else {
            res_fut.await
        }?;

        debug!(
            "subscription connection to subgraph {} at {} established, status: {}",
            self.subgraph_name,
            self.endpoint.to_string(),
            res.status()
        );

        // TODO: non-success statuses are not handled in single-shot results?
        //       seems like the body is read regardless there, do the same in subscriptions?
        // if !res.status().is_success() {
        //     return Err(SubgraphExecutorError::RequestFailure(...));
        // }

        let (parts, body_stream) = res.into_parts();
        let _response_headers = parts.headers.clone();

        let content_type = parts
            .headers
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let is_multipart = content_type.starts_with("multipart/mixed");
        let is_sse = content_type == "text/event-stream";

        if !is_multipart && !is_sse {
            return Err(SubgraphExecutorError::UnsupportedContentTypeError(
                content_type.to_string(),
                self.subgraph_name.clone(),
            ));
        }

        // clone to avoid borrowing self in stream closures
        let endpoint = self.endpoint.to_string();
        let subgraph_name = self.subgraph_name.clone();

        if is_multipart {
            debug!(
                subgraph_name = self.subgraph_name,
                "using multipart HTTP for subscription",
            );

            let boundary =
                multipart_subscribe::parse_boundary_from_header(content_type).map_err(|e| {
                    SubgraphExecutorError::SubscriptionStreamError(
                        self.endpoint.to_string(),
                        format!("Failed to parse boundary from Content-Type header: {}", e),
                    )
                })?;
            let stream = multipart_subscribe::parse_to_stream(boundary, body_stream);

            Ok(Box::pin(async_stream::stream! {
                trace!("multipart subscription stream started");
                for await result in stream {
                    match result {
                        Ok(response) => {
                            trace!(response = ?response, "multipart subscription event received");
                            yield response;
                        }
                        Err(e) => {
                            let error = SubgraphExecutorError::SubscriptionStreamError(
                                endpoint.clone(),
                                e.to_string(),
                            );
                            yield error.to_subgraph_response(subgraph_name.as_str());
                            return;
                        }
                    }
                }
            }))
        } else {
            debug!(
                "using SSE for subscription connection to subgraph {} at {}",
                self.subgraph_name,
                self.endpoint.to_string(),
            );

            let stream = sse::parse_to_stream(body_stream);

            Ok(Box::pin(async_stream::stream! {
                trace!("SSE subscription stream started");
                for await result in stream {
                    match result {
                        Ok(response) => {
                            trace!(response = ?response, "SSE subscription event received");
                            yield response;
                        }
                        Err(e) => {
                            let error = SubgraphExecutorError::SubscriptionStreamError(
                                endpoint.clone(),
                                e.to_string(),
                            );
                            yield error.to_subgraph_response(subgraph_name.as_str());
                            return;
                        }
                    }
                }
            }))
        }
    }
}

#[derive(Default, Clone)]
pub struct SubgraphHttpResponse {
    pub status: StatusCode,
    pub headers: Arc<HeaderMap>,
    pub body: Bytes,
}

impl SubgraphHttpResponse {
    fn deserialize_http_response<'a>(self) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
        SubgraphResponse::deserialize_from_bytes(self.body.clone()).map(
            |mut resp: SubgraphResponse<'a>| {
                // headers are under arc, zero cost clone
                resp.headers = Some(self.headers.clone());
                resp
            },
        )
    }
}

struct TraceHeaderInjector<'a>(pub &'a mut HeaderMap);

impl<'a> Injector for TraceHeaderInjector<'a> {
    fn set(&mut self, key: &str, value: String) {
        let Ok(name) = HeaderName::from_bytes(key.as_bytes()) else {
            return;
        };

        let Ok(val) = HeaderValue::from_str(&value) else {
            return;
        };

        self.0.insert(name, val);
    }
}
