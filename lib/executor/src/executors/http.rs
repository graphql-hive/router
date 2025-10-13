use std::sync::Arc;
use std::time::Duration;

use crate::executors::dedupe::{request_fingerprint, unique_leader_fingerprint};
use crate::executors::map::InflightRequestsMap;
use crate::hooks::on_subgraph_http_request::{
    OnSubgraphHttpRequestHookPayload, OnSubgraphHttpResponseHookPayload,
};
use crate::plugin_context::PluginRequestState;
use crate::plugin_trait::{EndControlFlow, StartControlFlow};
use crate::response::subgraph_response::SubgraphResponse;
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
    pub http_client: Arc<Client<HttpsConnector<HttpConnector>, Full<Bytes>>>,
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

struct SendRequestOpts<'a> {
    http_client: &'a Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    subgraph_name: &'a str,
    endpoint: &'a http::Uri,
    method: http::Method,
    body: Vec<u8>,
    execution_request: SubgraphExecutionRequest<'a>,
    plugin_req_state: &'a Option<PluginRequestState<'a>>,
    timeout: Option<Duration>,
    telemetry_context: Arc<TelemetryContext>,
}

async fn send_request<'a>(
    opts: SendRequestOpts<'a>,
) -> Result<SubgraphHttpResponse, SubgraphExecutorError> {
    let SendRequestOpts {
        http_client,
        subgraph_name,
        endpoint,
        mut method,
        mut body,
        mut execution_request,
        plugin_req_state,
        timeout,
        telemetry_context,
    } = opts;
    let mut on_end_callbacks = vec![];
    let mut response = None;

    if let Some(plugin_req_state) = plugin_req_state.as_ref() {
        let mut start_payload = OnSubgraphHttpRequestHookPayload {
            subgraph_name,
            endpoint,
            method,
            body,
            execution_request,
            context: &plugin_req_state.context,
        };
        for plugin in plugin_req_state.plugins.as_ref() {
            let result = plugin.on_subgraph_http_request(start_payload).await;
            start_payload = result.payload;
            match result.control_flow {
                StartControlFlow::Proceed => { /* continue to next plugin */ }
                StartControlFlow::EndWithResponse(early_response) => {
                    response = Some(early_response);
                    break;
                }
                StartControlFlow::OnEnd(callback) => {
                    on_end_callbacks.push(callback);
                }
            }
        }
        // Give the ownership back to variables
        method = start_payload.method;
        body = start_payload.body;
        execution_request = start_payload.execution_request;
    }

    let mut response = match response {
        Some(response) => response,
        None => {
            let mut req = hyper::Request::builder()
                .method(method)
                .uri(endpoint)
                .version(Version::HTTP_11)
                .body(Full::new(Bytes::from(body)))?;

            *req.headers_mut() = execution_request.headers;

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
                        .map_err(|_| {
                            SubgraphExecutorError::RequestTimeout(timeout_duration.as_millis())
                        })?
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
            .await?
        }
    };

    if !on_end_callbacks.is_empty() {
        if let Some(plugin_req_state) = plugin_req_state.as_ref() {
            let mut end_payload = OnSubgraphHttpResponseHookPayload {
                response,
                context: &plugin_req_state.context,
            };

            for callback in on_end_callbacks {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Proceed => { /* continue to next callback */ }
                    EndControlFlow::EndWithResponse(new_response) => {
                        end_payload.response = new_response;
                    }
                }
            }

            // Give the ownership back to variables
            response = end_payload.response;
        }
    }

    Ok(response)
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
        let body = self.build_request_body(&execution_request)?;

        self.header_map.iter().for_each(|(key, value)| {
            execution_request.headers.insert(key, value.clone());
        });

        let method = http::Method::POST;

        if !self.dedupe_enabled || !execution_request.dedupe {
            // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
            // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
            let _permit = self.semaphore.acquire().await.unwrap();
            let shared_response = send_request(SendRequestOpts {
                http_client: &self.http_client,
                subgraph_name: &self.subgraph_name,
                endpoint: &self.endpoint,
                method,
                body,
                execution_request,
                plugin_req_state,
                timeout,
                telemetry_context: self.telemetry_context.clone(),
            })
            .await?;

            return shared_response.deserialize_http_response();
        }

        let fingerprint =
            request_fingerprint(&method, &self.endpoint, &execution_request.headers, &body);
        let inflight_span = HttpInflightRequestSpan::new(
            &http::Method::POST,
            &self.endpoint,
            &execution_request.headers,
            &body,
        );

        async {
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
                        send_request(SendRequestOpts {
                            http_client: &self.http_client,
                            subgraph_name: &self.subgraph_name,
                            endpoint: &self.endpoint,
                            method,
                            body,
                            execution_request,
                            plugin_req_state,
                            timeout,
                            telemetry_context: self.telemetry_context.clone(),
                        })
                        .await
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

            inflight_span.record_response(&shared_response.body, &shared_response.status);

            shared_response.deserialize_http_response()
        }
        .instrument(inflight_span.clone())
        .await
    }
}

#[derive(Default, Clone)]
pub struct SubgraphHttpResponse {
    pub status: StatusCode,
    pub headers: Arc<HeaderMap>,
    pub body: Bytes,
}

impl SubgraphHttpResponse {
    fn deserialize_http_response<'a>(&self) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
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
                resp.headers = Some(self.headers.clone());
                // Zero cost of cloning Bytes
                resp.bytes = Some(self.body.clone());
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
