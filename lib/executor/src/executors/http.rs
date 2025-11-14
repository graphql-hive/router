use std::sync::Arc;

use crate::executors::common::HttpExecutionResponse;
use crate::executors::compression::CompressionType;
use crate::executors::dedupe::{request_fingerprint, ABuildHasher, SharedResponse};
use dashmap::DashMap;
use hive_router_config::HiveRouterConfig;
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

use crate::executors::common::HttpExecutionRequest;
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
    pub config: Arc<HiveRouterConfig>,
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
        config: Arc<HiveRouterConfig>,
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
        header_map.insert(
            http::header::ACCEPT_ENCODING,
            HeaderValue::from_static(CompressionType::accept_encoding()),
        );

        Self {
            subgraph_name,
            endpoint,
            http_client,
            header_map,
            semaphore,
            config,
            in_flight_requests,
        }
    }

    fn build_request_body<'a>(
        &self,
        execution_request: &HttpExecutionRequest<'a>,
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

        let res = self.http_client.request(req).await.map_err(|e| {
            SubgraphExecutorError::RequestFailure(self.endpoint.to_string(), e.to_string())
        })?;

        debug!(
            "http request to {} completed, status: {}",
            self.endpoint.to_string(),
            res.status()
        );

        let (parts, body) = res.into_parts();
        let mut body = body
            .collect()
            .await
            .map_err(|e| {
                SubgraphExecutorError::RequestFailure(self.endpoint.to_string(), e.to_string())
            })?
            .to_bytes();

        let content_encoding_header = parts
            .headers
            .get(http::header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok());

        if let Some(content_encoding_header) = content_encoding_header {
            let decompressor = CompressionType::from_encoding_header(content_encoding_header)?;

            body = decompressor.decompress(body).await?;
        }

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
    #[tracing::instrument(skip_all, fields(subgraph_name = self.subgraph_name))]
    async fn execute<'a>(
        &self,
        execution_request: HttpExecutionRequest<'a>,
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

        if !self.config.traffic_shaping.dedupe_enabled || !execution_request.dedupe {
            // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
            // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
            let _permit = self.semaphore.acquire().await.unwrap();
            return match self._send_request(body, headers).await {
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
                    self._send_request(body, headers).await
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
}
