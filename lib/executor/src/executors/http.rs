use std::sync::Arc;
use std::time::Duration;

use crate::executors::common::HttpExecutionResponse;
use crate::executors::dedupe::{request_fingerprint, ABuildHasher, SharedResponse};
use crate::executors::sse;
use dashmap::DashMap;
use futures::stream::BoxStream;
use futures_util::{stream, TryFutureExt};
use tokio::sync::OnceCell;

use async_trait::async_trait;

use bytes::{BufMut, Bytes, BytesMut};
use http::HeaderMap;
use http::HeaderValue;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Version;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use tokio::sync::Semaphore;
use tracing::debug;

use crate::executors::common::SubgraphExecutionRequest;
use crate::executors::error::SubgraphExecutorError;
use crate::response::graphql_error::GraphQLError;
use crate::utils::consts::CLOSE_BRACE;
use crate::utils::consts::COLON;
use crate::utils::consts::COMMA;
use crate::utils::consts::QUOTE;
use crate::{executors::common::SubgraphExecutor, json_writer::write_and_escape_string};

#[derive(Debug)]
pub struct HTTPSubgraphExecutor {
    pub subgraph_name: String,
    pub endpoint: http::Uri,
    pub http_client: Arc<Client<HttpsConnector<HttpConnector>, Full<Bytes>>>,
    pub header_map: HeaderMap,
    pub semaphore: Arc<Semaphore>,
    pub dedupe_enabled: bool,
    pub in_flight_requests: Arc<DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>>,
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
        in_flight_requests: Arc<DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>>,
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
        }
    }

    fn build_request_body(
        &self,
        execution_request: &SubgraphExecutionRequest<'_>,
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

    async fn _send_request(
        &self,
        body: Vec<u8>,
        headers: HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<SharedResponse, SubgraphExecutorError> {
        let mut req = hyper::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .version(Version::HTTP_11)
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| {
                SubgraphExecutorError::RequestBuildFailure(self.endpoint.to_string(), e.to_string())
            })?;

        *req.headers_mut() = headers;

        debug!("making http request to {}", self.endpoint.to_string());

        let res_fut = self.http_client.request(req).map_err(|e| {
            SubgraphExecutorError::RequestFailure(self.endpoint.to_string(), e.to_string())
        });

        let res = if let Some(timeout_duration) = timeout {
            tokio::time::timeout(timeout_duration, res_fut)
                .await
                .map_err(|_| {
                    SubgraphExecutorError::RequestTimeout(
                        self.endpoint.to_string(),
                        timeout_duration.as_millis(),
                    )
                })?
        } else {
            res_fut.await
        }?;

        debug!(
            "http request to {} completed, status: {}",
            self.endpoint.to_string(),
            res.status()
        );

        let (parts, body) = res.into_parts();
        let body = body
            .collect()
            .await
            .map_err(|e| {
                SubgraphExecutorError::RequestFailure(self.endpoint.to_string(), e.to_string())
            })?
            .to_bytes();

        if body.is_empty() {
            return Err(SubgraphExecutorError::RequestFailure(
                self.endpoint.to_string(),
                "Empty response body".to_string(),
            ));
        }

        Ok(SharedResponse {
            status: parts.status,
            body,
            headers: parts.headers,
        })
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

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    #[tracing::instrument(level = "trace", skip_all, fields(subgraph_name = %self.subgraph_name))]
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> HttpExecutionResponse {
        let body = match self.build_request_body(&execution_request) {
            Ok(body) => body,
            Err(e) => {
                self.log_error(&e);
                return HttpExecutionResponse {
                    body: self.error_to_graphql_bytes(e),
                    headers: Default::default(),
                };
            }
        };

        let mut headers = execution_request.headers;
        self.header_map.iter().for_each(|(key, value)| {
            headers.insert(key, value.clone());
        });

        if !self.dedupe_enabled || !execution_request.dedupe {
            // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
            // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
            let _permit = self.semaphore.acquire().await.unwrap();
            return match self._send_request(body, headers, timeout).await {
                Ok(shared_response) => HttpExecutionResponse {
                    body: shared_response.body,
                    headers: shared_response.headers,
                },
                Err(e) => {
                    self.log_error(&e);
                    HttpExecutionResponse {
                        body: self.error_to_graphql_bytes(e),
                        headers: Default::default(),
                    }
                }
            };
        }

        let fingerprint = request_fingerprint(&http::Method::POST, &self.endpoint, &headers, &body);

        // Clone the cell from the map, dropping the lock from the DashMap immediately.
        // Prevents any deadlocks.
        let cell = self
            .in_flight_requests
            .entry(fingerprint)
            .or_default()
            .value()
            .clone();

        let response_result = cell
            .get_or_try_init(|| async {
                let res = {
                    // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
                    // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
                    let _permit = self.semaphore.acquire().await.unwrap();
                    self._send_request(body, headers, timeout).await
                };
                // It's important to remove the entry from the map before returning the result.
                // This ensures that once the OnceCell is set, no future requests can join it.
                // The cache is for the lifetime of the in-flight request only.
                self.in_flight_requests.remove(&fingerprint);
                res
            })
            .await;

        match response_result {
            Ok(shared_response) => HttpExecutionResponse {
                body: shared_response.body.clone(),
                headers: shared_response.headers.clone(),
            },
            Err(e) => {
                self.log_error(&e);
                HttpExecutionResponse {
                    body: self.error_to_graphql_bytes(e.clone()),
                    headers: Default::default(),
                }
            }
        }
    }

    #[tracing::instrument(level = "trace", skip_all, fields(subgraph_name = %self.subgraph_name))]
    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        connection_timeout: Option<Duration>,
    ) -> BoxStream<'static, HttpExecutionResponse> {
        // NOTE: we intentionally stream once errors. read mroe in https://github.com/enisdenjo/graphql-sse/blob/master/PROTOCOL.md#distinct-connections-mode

        // is this heavy in rust? I dont see using closures elsewhere much
        // TODO: convert to function
        let stream_once_err = |err: SubgraphExecutorError| {
            self.log_error(&err);
            let error_bytes = self.error_to_graphql_bytes(err);
            return Box::pin(stream::once(async move {
                HttpExecutionResponse {
                    body: error_bytes,
                    headers: Default::default(),
                }
            }));
        };

        let body = match self.build_request_body(&execution_request) {
            Ok(body) => body,
            Err(e) => {
                return stream_once_err(e);
            }
        };

        let mut req = match hyper::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .version(Version::HTTP_11)
            .body(Full::new(Bytes::from(body)))
        {
            Ok(req) => req,
            Err(e) => {
                return stream_once_err(SubgraphExecutorError::RequestBuildFailure(
                    self.endpoint.to_string(),
                    e.to_string(),
                ));
            }
        };

        let mut headers = execution_request.headers;
        self.header_map.iter().for_each(|(key, value)| {
            headers.insert(key, value.clone());
        });

        // accept text/event-stream
        // TODO: multipart
        headers.insert(
            http::header::ACCEPT,
            HeaderValue::from_static(r#"text/event-stream"#),
        );
        *req.headers_mut() = headers;

        debug!(
            "establishing subscription connection to subgraph {} at {}",
            self.subgraph_name,
            self.endpoint.to_string()
        );

        let req_fut = self.http_client.request(req);
        let res = if let Some(timeout_duration) = connection_timeout {
            match tokio::time::timeout(timeout_duration, req_fut).await {
                Ok(Ok(response)) => {
                    // good
                    response
                }
                Ok(Err(e)) => {
                    // request error
                    return stream_once_err(SubgraphExecutorError::RequestFailure(
                        self.endpoint.to_string(),
                        e.to_string(),
                    ));
                }
                Err(_) => {
                    // connection timeout
                    return stream_once_err(SubgraphExecutorError::RequestTimeout(
                        self.endpoint.to_string(),
                        timeout_duration.as_millis(),
                    ));
                }
            }
        } else {
            match req_fut.await {
                Ok(response) => response,
                Err(e) => {
                    return stream_once_err(SubgraphExecutorError::RequestFailure(
                        self.endpoint.to_string(),
                        e.to_string(),
                    ));
                }
            }
        };

        debug!(
            "subscription connection to subgraph {} at {} established, status: {}",
            self.subgraph_name,
            self.endpoint.to_string(),
            res.status()
        );

        if !res.status().is_success() {
            // TODO: how are non-success statuses handled in single-shot results?
            //       seems like the body is read regardless
            return stream_once_err(SubgraphExecutorError::RequestFailure(
                self.endpoint.to_string(),
                format!("Subgraph returned non-success status: {}", res.status()),
            ));
        }

        let (parts, body_stream) = res.into_parts();
        let response_headers = parts.headers.clone();
        let subgraph_name = self.subgraph_name.clone();

        let content_type = parts
            .headers
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if content_type != "text/event-stream" {
            return stream_once_err(SubgraphExecutorError::UnsupportedContentTypeError(
                content_type.to_string(),
                self.subgraph_name.clone(),
            ));
        }

        let stream = sse::parse_to_stream(body_stream);
        let endpoint = self.endpoint.to_string();

        Box::pin(async_stream::stream! {
            for await result in stream {
                match result {
                    Ok(body) => {
                        yield HttpExecutionResponse {
                            body,
                            headers: response_headers.clone(),
                        };
                    }
                    Err(e) => {
                        // TODO: I cannot reuse self.log_error here because of 'self' move issues
                        //       is there a nicer way to do this without repetition?
                        let error = SubgraphExecutorError::SubscriptionStreamError(
                            endpoint.clone(),
                            e.to_string(),
                        );

                        // self.log_error(&e);
                        tracing::error!(
                            error = &error as &dyn std::error::Error,
                            "Subgraph executor error"
                        );

                        // let error_bytes = self.error_to_graphql_bytes(e);
                        let graphql_error: GraphQLError = error.into();
                        let graphql_error = graphql_error.add_subgraph_name(&subgraph_name);
                        let errors = vec![graphql_error];
                        let errors_bytes = sonic_rs::to_vec(&errors).unwrap();
                        let mut buffer = BytesMut::new();
                        buffer.put_slice(b"{\"errors\":");
                        buffer.put_slice(&errors_bytes);
                        buffer.put_slice(b"}");
                        let error_bytes = buffer.freeze();

                        yield HttpExecutionResponse {
                            body: error_bytes,
                            headers: response_headers.clone(),
                        };
                        return;
                    }
                }
            }
        })
    }
}
