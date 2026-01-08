use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::executors::dedupe::unique_leader_fingerprint;
use crate::executors::map::InflightRequestsMap;
use crate::hooks::on_subgraph_http_request::{
    OnSubgraphHttpRequestHookPayload, OnSubgraphHttpResponseHookPayload,
};
use crate::plugin_context::PluginRequestState;
use crate::plugin_trait::{EndControlFlow, StartControlFlow};
use crate::response::subgraph_response::SubgraphResponse;
use hive_router_config::HiveRouterConfig;
use hive_router_internal::telemetry::metrics::catalog::values::GraphQLResponseStatus;
use hive_router_internal::telemetry::metrics::http_client_metrics::HttpClientRequestStateCapture;
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
use tracing::debug;

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

struct FetchedSubgraphResponse<'a> {
    response: SubgraphHttpResponse,
    http_request_capture: HttpClientRequestStateCapture<'a>,
    transport_duration: Duration,
}

struct HttpRequestTelemetryCapture<'a> {
    capture: HttpClientRequestStateCapture<'a>,
    response_body_size: u64,
    transport_duration: Duration,
}

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
    pub subgraph_name: &'a str,
    pub method: http::Method,
    pub body: Vec<u8>,
    pub headers: HeaderMap,
    pub timeout: Option<Duration>,
    pub telemetry_context: &'a Arc<TelemetryContext>,
}

async fn send_request<'a>(
    opts: SendRequestOpts<'a>,
) -> Result<FetchedSubgraphResponse<'a>, SubgraphExecutorError> {
    let SendRequestOpts {
        http_client,
        endpoint,
        subgraph_name,
        method,
        body,
        headers,
        timeout,
        telemetry_context,
    } = opts;
    let request_body_size = body.len() as u64;

    let mut req = hyper::Request::builder()
        .method(method)
        .uri(endpoint)
        .version(Version::HTTP_11)
        .body(Full::new(Bytes::from(body)))?;

    *req.headers_mut() = headers;

    debug!("making http request to {}", endpoint.to_string());

    let http_request_span = HttpClientRequestSpan::from_request(&req);
    let mut http_request_capture = telemetry_context.metrics.http_client.capture_request(
        &req,
        request_body_size,
        Some(subgraph_name),
    );
    let transport_started_at = Instant::now();

    let response: Result<SubgraphHttpResponse, SubgraphExecutorError> = async {
        // TODO: let's decide at some point if the tracing headers
        //       should be part of the fingerprint or not.
        telemetry_context.inject_context(&mut TraceHeaderInjector(req.headers_mut()));

        let res_fut = http_client.request(req);

        let res = if let Some(timeout_duration) = timeout {
            tokio::time::timeout(timeout_duration, res_fut)
                .await
                .map_err(|_| {
                    SubgraphExecutorError::RequestTimeout(
                        endpoint.to_string(),
                        timeout_duration.as_millis(),
                    )
                })?
        } else {
            res_fut.await
        }?;

        http_request_span.record_response(&res);
        http_request_capture.set_status_code(res.status().as_u16());

        debug!(
            "http request to {} completed, status: {}",
            endpoint.to_string(),
            res.status()
        );

        let (parts, body) = res.into_parts();
        let body = body
            .collect()
            .await
            .map_err(|err| {
                SubgraphExecutorError::ResponseBodyReadFailure(
                    endpoint.to_string(),
                    err.to_string(),
                )
            })?
            .to_bytes();

        if body.is_empty() {
            return Err(SubgraphExecutorError::EmptyResponseBody(
                subgraph_name.to_string(),
            ));
        }

        Ok(SubgraphHttpResponse {
            status: parts.status,
            body,
            headers: parts.headers.into(),
        })
    }
    .instrument(http_request_span.clone())
    .await;

    let transport_duration = transport_started_at.elapsed();

    match response {
        Ok(response) => Ok(FetchedSubgraphResponse {
            response,
            http_request_capture,
            transport_duration,
        }),
        Err(err) => {
            http_request_capture.finish_error(err.error_code(), transport_duration);
            Err(err)
        }
    }
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
        let mut http_request_capture = None;

        let mut response = match response {
            Some(resp) => resp,
            None => {
                let send_request_opts = SendRequestOpts {
                    http_client: &self.http_client,
                    endpoint: &self.endpoint,
                    subgraph_name: &self.subgraph_name,
                    method,
                    body,
                    headers: execution_request.headers,
                    timeout,
                    telemetry_context: &self.telemetry_context,
                };

                if deduplicate_request {
                    // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
                    // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
                    let _permit = self.semaphore.acquire().await.unwrap();
                    let fetched_response = send_request(send_request_opts).await?;
                    http_request_capture = Some(HttpRequestTelemetryCapture {
                        capture: fetched_response.http_request_capture,
                        response_body_size: fetched_response.response.body.len() as u64,
                        transport_duration: fetched_response.transport_duration,
                    });
                    fetched_response.response
                } else {
                    let fingerprint = send_request_opts.fingerprint();

                    let inflight_span = HttpInflightRequestSpan::new(
                        &send_request_opts.method,
                        send_request_opts.endpoint,
                        &send_request_opts.headers,
                        &send_request_opts.body,
                    );

                    let result: Result<_, SubgraphExecutorError> = async {
                        // Clone the cell from the map, dropping the lock from the DashMap immediately.
                        // Prevents any deadlocks.
                        let cell = self
                            .in_flight_requests
                            .entry(fingerprint)
                            .or_default()
                            .clone();
                        // Mark it as a joiner span by default.
                        let mut is_leader = false;
                        let mut leader_http_request_capture = None;
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
                                res.map(|fetched_response| {
                                    leader_http_request_capture =
                                        Some(HttpRequestTelemetryCapture {
                                            capture: fetched_response.http_request_capture,
                                            response_body_size: fetched_response.response.body.len()
                                                as u64,
                                            transport_duration: fetched_response.transport_duration,
                                        });
                                    (fetched_response.response, unique_leader_fingerprint())
                                })
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

                        Ok((shared_response.clone(), leader_http_request_capture))
                    }
                    .instrument(inflight_span.clone())
                    .await;

                    let (shared_response, leader_http_request_capture) = result?;
                    if let Some(capture) = leader_http_request_capture {
                        http_request_capture = Some(capture);
                    }

                    shared_response
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

        let response_result = response.deserialize_http_response();
        if let Some(mut http_request_capture) = http_request_capture {
            finish_capture_from_subgraph_result(
                &mut http_request_capture.capture,
                http_request_capture.response_body_size,
                http_request_capture.transport_duration,
                &response_result,
            );
        }

        response_result
    }
}

fn finish_capture_from_subgraph_result(
    capture: &mut HttpClientRequestStateCapture<'_>,
    response_body_size: u64,
    transport_duration: Duration,
    response_result: &Result<SubgraphResponse<'_>, SubgraphExecutorError>,
) {
    let error_code = response_result.as_ref().err().map(|err| err.error_code());
    let graphql_response_status = if response_result.as_ref().is_ok_and(|response| {
        response
            .errors
            .as_ref()
            .is_none_or(|errors| errors.is_empty())
    }) {
        GraphQLResponseStatus::Ok
    } else {
        GraphQLResponseStatus::Error
    };

    capture.finish(
        response_body_size,
        transport_duration,
        graphql_response_status,
        error_code,
    );
}

#[derive(Default, Clone)]
pub struct SubgraphHttpResponse {
    pub status: StatusCode,
    pub headers: Arc<HeaderMap>,
    pub body: Bytes,
}

impl SubgraphHttpResponse {
    fn deserialize_http_response<'a>(self) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
        let bytes_ref: &[u8] = &self.body;

        // SAFETY: The byte slice `bytes_ref` is transmuted to have lifetime `'a`.
        // This is safe because the returned `SubgraphResponse` contains a clone of `self.body`
        // in its `bytes` field. `Bytes` is a reference-counted buffer, so this ensures the
        // underlying data remains alive as long as the `SubgraphResponse` does.
        // The `data` field of `SubgraphResponse` contains values that borrow from this buffer,
        // creating a self-referential struct, which is why `unsafe` is required.
        let bytes_ref: &'a [u8] = unsafe { std::mem::transmute(bytes_ref) };

        sonic_rs::from_slice(bytes_ref)
            .map_err(SubgraphExecutorError::ResponseDeserializationFailure)
            .map(|mut resp: SubgraphResponse<'a>| {
                // This is Arc
                resp.headers = Some(self.headers);
                // Zero cost of cloning Bytes
                resp.bytes = Some(self.body);
                resp
            })
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
