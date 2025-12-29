use std::sync::Arc;
use std::time::Duration;

use crate::executors::dedupe::request_fingerprint;
use crate::executors::map::InflightRequestsMap;
use crate::hooks::on_subgraph_http_request::{
    OnSubgraphHttpRequestHookPayload, OnSubgraphHttpResponseHookPayload,
};
use crate::plugin_context::PluginRequestState;
use crate::plugin_trait::{EndControlFlow, StartControlFlow};
use crate::response::subgraph_response::{SubgraphResponse, SubgraphResponseDeserialized};
use crate::response::value::Value;
use futures::TryFutureExt;
use hive_router_config::HiveRouterConfig;

use async_trait::async_trait;

use bytes::{BufMut, Bytes, BytesMut};
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
use crate::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};
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

    fn error_to_graphql_bytes(&self, error: SubgraphExecutorError) -> Bytes {
        let graphql_error: GraphQLError = error.into();
        let mut graphql_error = graphql_error.add_subgraph_name(&self.subgraph_name);
        graphql_error.message = "Failed to execute request to subgraph".to_string();

        let errors = vec![graphql_error];
        // This unwrap is safe as GraphQLError serialization shouldn't fail.
        let errors_bytes = sonic_rs::to_vec(&errors).unwrap();
        let mut buffer = BytesMut::new();
        buffer.put_slice(b"{\"errors\":");
        buffer.put_slice(&errors_bytes);
        buffer.put_slice(b"}");
        buffer.freeze()
    }

    fn log_error(&self, error: &SubgraphExecutorError) {
        tracing::error!(
            error = error as &dyn std::error::Error,
            "Subgraph executor error"
        );
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
) -> Result<Arc<HttpResponse>, SubgraphExecutorError> {
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
            response,
        };
        for plugin in plugin_req_state.plugins.as_ref() {
            let result = plugin.on_subgraph_http_request(start_payload).await;
            start_payload = result.payload;
            match result.control_flow {
                StartControlFlow::Continue => { /* continue to next plugin */ }
                StartControlFlow::EndResponse(response) => {
                    return Ok(response.into());
                }
                StartControlFlow::OnEnd(callback) => {
                    on_end_callbacks.push(callback);
                }
            }
        }
        method = start_payload.method;
        body = start_payload.body;
        execution_request = start_payload.execution_request;
        response = start_payload.response;
    }

    let mut response = match response {
        Some(response) => response,
        None => {
            let mut req = hyper::Request::builder()
                .method(method)
                .uri(endpoint)
                .version(Version::HTTP_11)
                .body(Full::new(Bytes::from(body)))
                .map_err(|e| {
                    SubgraphExecutorError::RequestBuildFailure(endpoint.to_string(), e.to_string())
                })?;

            *req.headers_mut() = execution_request.headers;

            debug!("making http request to {}", endpoint.to_string());

            let res_fut = http_client.request(req).map_err(|e| {
                SubgraphExecutorError::RequestFailure(endpoint.to_string(), e.to_string())
            });

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

            debug!(
                "http request to {} completed, status: {}",
                endpoint.to_string(),
                res.status()
            );

            let (parts, body) = res.into_parts();
            let body = body
                .collect()
                .await
                .map_err(|e| {
                    SubgraphExecutorError::RequestFailure(endpoint.to_string(), e.to_string())
                })?
                .to_bytes();

            if body.is_empty() {
                return Err(SubgraphExecutorError::RequestFailure(
                    endpoint.to_string(),
                    "Empty response body".to_string(),
                ));
            }

            HttpResponse {
                status: parts.status,
                body: body.into(),
                headers: parts.headers.into(),
            }
            .into()
        }
    };

    if let Some(plugin_req_state) = plugin_req_state.as_ref() {
        let mut end_payload = OnSubgraphHttpResponseHookPayload {
            response,
            context: &plugin_req_state.context,
        };

        for callback in on_end_callbacks {
            let result = callback(end_payload);
            end_payload = result.payload;
            match result.control_flow {
                EndControlFlow::Continue => { /* continue to next callback */ }
                EndControlFlow::EndResponse(response) => {
                    return Ok(response.into());
                }
            }
        }

        response = end_payload.response;
    }

    Ok(response)
}

impl HTTPSubgraphExecutor {
    #[tracing::instrument(level = "trace", skip_all, fields(subgraph_name = %self.subgraph_name))]
    async fn execute_http<'a>(
        &self,
        mut execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: &'a Option<PluginRequestState<'a>>,
    ) -> Arc<HttpResponse> {
        let body = match self.build_request_body(&execution_request) {
            Ok(body) => body,
            Err(e) => {
                self.log_error(&e);
                return HttpResponse {
                    body: self.error_to_graphql_bytes(e).into(),
                    headers: Default::default(),
                    status: StatusCode::OK,
                }
                .into();
            }
        };

        self.header_map.iter().for_each(|(key, value)| {
            execution_request.headers.insert(key, value.clone());
        });

        let method = http::Method::POST;

        if !self.dedupe_enabled || !execution_request.dedupe {
            // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
            // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
            let _permit = self.semaphore.acquire().await.unwrap();

            return match send_request(SendRequestOpts {
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
            {
                Ok(shared_response) => shared_response,
                Err(e) => {
                    self.log_error(&e);
                    HttpResponse {
                        body: self.error_to_graphql_bytes(e).into(),
                        headers: Default::default(),
                        status: StatusCode::OK,
                    }
                    .into()
                }
            };
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

        let response_result = cell
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
            .await;

        match response_result {
            Ok(shared_response) => shared_response.clone(),
            Err(e) => {
                self.log_error(&e);
                HttpResponse {
                    body: self.error_to_graphql_bytes(e).into(),
                    headers: Default::default(),
                    status: StatusCode::OK,
                }
                .into()
            }
        }
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    fn endpoint(&self) -> &http::Uri {
        &self.endpoint
    }
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: &'a Option<PluginRequestState<'a>>,
    ) -> SubgraphResponse<'a> {
        let http_response = self
            .execute_http(execution_request, timeout, plugin_req_state)
            .await;

        http_response.into()
    }
}

#[derive(Clone)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: Arc<HeaderMap>,
    pub body: Arc<Bytes>,
}

impl<'a> From<Arc<HttpResponse>> for SubgraphResponse<'a> {
    fn from(http_response: Arc<HttpResponse>) -> Self {
        let bytes: &[u8] = &http_response.body;

        // SAFETY: The `bytes` are transmuted to the lifetime `'a` of the `ExecutionContext`.
        // This is safe because the `response_storage` is part of the `ExecutionContext` (`ctx`)
        // and will live as long as `'a`. The `Bytes` are stored in an `Arc`, so they won't be
        // dropped until all references are gone. The `Value`s deserialized from this byte
        // slice will borrow from it, and they are stored in `ctx.final_response`, which also
        // lives for `'a`.
        let bytes: &'a [u8] = unsafe { std::mem::transmute(bytes) };

        let subgraph_res_deserialized: Result<SubgraphResponseDeserialized<'a>, GraphQLError> =
            sonic_rs::from_slice(bytes).map_err(|e| {
                let message = format!("Failed to deserialize subgraph response: {}", e);
                let extensions = GraphQLErrorExtensions::new_from_code(
                    "SUBGRAPH_RESPONSE_DESERIALIZATION_FAILED",
                );
                GraphQLError::from_message_and_extensions(message, extensions)
            });

        let headers = Some(http_response.headers.clone());
        let bytes = Some(http_response.body.clone());

        match subgraph_res_deserialized {
            Ok(res) => SubgraphResponse {
                data: res.data,
                errors: res.errors,
                extensions: res.extensions,
                headers,
                bytes,
            },
            Err(e) => SubgraphResponse {
                data: Value::Null,
                errors: Some(vec![e]),
                extensions: None,
                headers,
                bytes,
            },
        }
    }
}
