use std::sync::Arc;
use std::time::Duration;

use crate::executors::dedupe::request_fingerprint;
use crate::executors::map::InflightRequestsMap;
use crate::response::subgraph_response::SubgraphResponse;
use futures::TryFutureExt;

use async_trait::async_trait;

use bytes::{BufMut, Bytes};
use hive_router_internal::telemetry::traces::spans::http_request::HttpClientRequestSpan;
use hive_router_internal::telemetry::traces::spans::http_request::HttpInflightRequestSpan;
use hive_router_internal::telemetry::Injector;
use hive_router_internal::telemetry::TelemetryContext;
use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;
use http::StatusCode;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Version;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use tokio::sync::Semaphore;
use tracing::debug;
use tracing::Instrument;

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
    pub in_flight_requests: InflightRequestsMap,
    pub telemetry_context: Arc<TelemetryContext>,
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
        in_flight_requests: InflightRequestsMap,
        telemetry_context: Arc<TelemetryContext>,
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

    async fn _send_request(
        &self,
        body: Vec<u8>,
        headers: HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<HttpResponse, SubgraphExecutorError> {
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

        let http_request_span = HttpClientRequestSpan::from_request(&req);

        async {
            // TODO: let's decide at some point if the tracing headers
            //       should be part of the fingerprint or not.
            self.telemetry_context
                .inject_context(&mut TraceHeaderInjector(req.headers_mut()));
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

            http_request_span.record_response(&res);

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

            Ok(HttpResponse {
                status: parts.status,
                body,
                headers: parts.headers.into(),
            })
        }
        .instrument(http_request_span.clone())
        .await
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
        let body = self.build_request_body(&execution_request)?;

        let mut headers = execution_request.headers;
        self.header_map.iter().for_each(|(key, value)| {
            headers.insert(key, value.clone());
        });

        if !self.dedupe_enabled || !execution_request.dedupe {
            // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
            // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
            let _permit = self.semaphore.acquire().await.unwrap();
            let shared_response = self._send_request(body, headers, timeout).await?;
            return shared_response.deserialize_http_response();
        }

        let fingerprint = request_fingerprint(&http::Method::POST, &self.endpoint, &headers, &body);
        let inflight_span = HttpInflightRequestSpan::new(
            &http::Method::POST,
            &self.endpoint,
            &headers,
            &body,
            fingerprint,
        );

        async {
            // Clone the cell from the map, dropping the lock from the DashMap immediately.
            // Prevents any deadlocks.
            let cell = self
                .in_flight_requests
                .entry(fingerprint)
                .or_default()
                .value()
                .clone();

            // Mark it as a joiner span by default.
            let mut is_leader = false;
            let shared_response = cell
                .get_or_try_init(|| async {
                    // Override the span to be a leader span for this request.
                    is_leader = true;
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
                .await?;

            if is_leader {
                inflight_span.record_as_leader();
            } else {
                inflight_span.record_as_joiner();
            }

            inflight_span.record_response(&shared_response.body, &shared_response.status);

            shared_response.deserialize_http_response()
        }
        .instrument(inflight_span.clone())
        .await
    }
}

#[derive(Default, Clone)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: Arc<HeaderMap>,
    pub body: Bytes,
}

impl HttpResponse {
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
            .map_err(|err| SubgraphExecutorError::ResponseDeserializationFailure(err.to_string()))
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
