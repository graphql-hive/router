use std::sync::Arc;
use std::time::Duration;

use crate::executors::dedupe::request_fingerprint;
use crate::executors::map::InflightRequestsMap;
use crate::hooks::on_subgraph_http_request::{
    OnSubgraphHttpRequestHookPayload, OnSubgraphHttpResponseHookPayload,
};
use crate::plugin_context::PluginRequestState;
use crate::plugin_trait::{EndControlFlow, StartControlFlow};
use crate::response::subgraph_response::SubgraphResponse;
use futures::TryFutureExt;
use hive_router_config::HiveRouterConfig;

use async_trait::async_trait;

use bytes::{BufMut, Bytes};
use http::HeaderValue;
use http::{HeaderMap, StatusCode};
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Version;
use hyper_tls::HttpsConnector;
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

pub struct HTTPSubgraphExecutor {
    pub subgraph_name: String,
    pub endpoint: http::Uri,
    pub http_client: Arc<Client<HttpsConnector<HttpConnector>, Full<Bytes>>>,
    pub header_map: HeaderMap,
    pub semaphore: Arc<Semaphore>,
    pub dedupe_enabled: bool,
    pub config: Arc<HiveRouterConfig>,
    pub in_flight_requests: InflightRequestsMap,
}

const FIRST_VARIABLE_STR: &[u8] = b",\"variables\":{";
const FIRST_QUOTE_STR: &[u8] = b"{\"query\":";

pub type HttpClient = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

impl HTTPSubgraphExecutor {
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        http_client: Arc<HttpClient>,
        semaphore: Arc<Semaphore>,
        dedupe_enabled: bool,
        config: Arc<HiveRouterConfig>,
        in_flight_requests: InflightRequestsMap,
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
                        err.to_string(),
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
}

async fn send_request<'a>(
    opts: SendRequestOpts<'a>,
) -> Result<HttpResponse, SubgraphExecutorError> {
    let SendRequestOpts {
        http_client,
        subgraph_name,
        endpoint,
        mut method,
        mut body,
        mut execution_request,
        plugin_req_state,
        timeout,
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
                .body(Full::new(Bytes::from(body)))
                .map_err(|e| SubgraphExecutorError::RequestBuildFailure(e.to_string()))?;

            *req.headers_mut() = execution_request.headers;

            debug!("making http request to {}", endpoint.to_string());

            let res_fut = http_client
                .request(req)
                .map_err(|e| SubgraphExecutorError::RequestFailure(e.to_string()));

            let res = if let Some(timeout_duration) = timeout {
                tokio::time::timeout(timeout_duration, res_fut)
                    .await
                    .map_err(|_| {
                        SubgraphExecutorError::RequestTimeout(timeout_duration.as_millis())
                    })?
            } else {
                res_fut.await
            }?;

            debug!(
                "http request to {} completed, status: {}",
                endpoint.to_string(),
                res.status()
            );

            let (parts, body) = res.into_parts();
            let body = body
                .collect()
                .await
                .map_err(|e| SubgraphExecutorError::ResponseFailure(e.to_string()))?
                .to_bytes();

            if body.is_empty() {
                return Err(SubgraphExecutorError::ResponseFailure(
                    "Empty response body".to_string(),
                ));
            }

            HttpResponse {
                status: parts.status,
                body: body.into(),
                headers: parts.headers.into(),
            }
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

            let http_response = send_request(SendRequestOpts {
                http_client: &self.http_client,
                subgraph_name: &self.subgraph_name,
                endpoint: &self.endpoint,
                method,
                body,
                execution_request,
                plugin_req_state,
                timeout,
            })
            .await?;

            return http_response.deserialize_http_response();
        }

        let fingerprint =
            request_fingerprint(&method, &self.endpoint, &execution_request.headers, &body);

        // Clone the cell from the map, dropping the lock from the DashMap immediately.
        // Prevents any deadlocks.
        let cell = self
            .in_flight_requests
            .entry(fingerprint)
            .or_default()
            .clone();

        let http_response = cell
            .get_or_try_init(|| async {
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
                    })
                    .await
                };
                // It's important to remove the entry from the map before returning the result.
                // This ensures that once the OnceCell is set, no future requests can join it.
                // The cache is for the lifetime of the in-flight request only.
                self.in_flight_requests.remove(&fingerprint);
                res
            })
            .await?;

        http_response.deserialize_http_response()
    }
}

#[derive(Default)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: Arc<HeaderMap>,
    pub body: Arc<Bytes>,
}

// Zero-cost clone
impl Clone for HttpResponse {
    fn clone(&self) -> Self {
        Self {
            status: self.status,
            headers: Arc::clone(&self.headers),
            body: Arc::clone(&self.body),
        }
    }
}

impl HttpResponse {
    fn deserialize_http_response<'a>(&self) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
        let bytes_ref: &[u8] = &self.body;

        // SAFETY: The `bytes` are transmuted to the lifetime `'a` of the `ExecutionContext`.
        // This is safe because the `response_storage` is part of the `ExecutionContext` (`ctx`)
        // and will live as long as `'a`. The `Bytes` are stored in an `Arc`, so they won't be
        // dropped until all references are gone. The `Value`s deserialized from this byte
        // slice will borrow from it, and they are stored in `ctx.final_response`, which also
        // lives for `'a`.
        let bytes_ref: &'a [u8] = unsafe { std::mem::transmute(bytes_ref) };

        sonic_rs::from_slice(bytes_ref)
            .map_err(|err| SubgraphExecutorError::ResponseDeserializationFailure(err.to_string()))
            .map(|mut resp: SubgraphResponse<'a>| {
                resp.headers = Some(self.headers.clone());
                resp.bytes = Some(self.body.clone());
                resp
            })
    }
}
