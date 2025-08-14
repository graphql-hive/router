use std::sync::Arc;

use crate::executors::config::HttpExecutorConfig;
use crate::executors::dedupe::{ABuildHasher, RequestFingerprint, SharedResponse};
use dashmap::DashMap;
use tokio::sync::OnceCell;

use async_trait::async_trait;

use bytes::{BufMut, BytesMut};
use http::HeaderMap;
use http::HeaderValue;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{body::Bytes, Version};
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use tokio::sync::Semaphore;

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
    pub endpoint: http::Uri,
    pub http_client: Arc<Client<HttpConnector, Full<Bytes>>>,
    pub header_map: HeaderMap,
    pub semaphore: Arc<Semaphore>,
    pub config: Arc<HttpExecutorConfig>,
    pub in_flight_requests:
        Arc<DashMap<RequestFingerprint, Arc<OnceCell<SharedResponse>>, ABuildHasher>>,
}

const FIRST_VARIABLE_STR: &[u8] = b",\"variables\":{";
const FIRST_QUOTE_STR: &[u8] = b"{\"query\":";

impl HTTPSubgraphExecutor {
    pub fn new(
        endpoint: http::Uri,
        http_client: Arc<Client<HttpConnector, Full<Bytes>>>,
        semaphore: Arc<Semaphore>,
        config: Arc<HttpExecutorConfig>,
        in_flight_requests: Arc<
            DashMap<RequestFingerprint, Arc<OnceCell<SharedResponse>>, ABuildHasher>,
        >,
    ) -> Self {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            "Content-Type",
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        header_map.insert(
            http::header::CONNECTION,
            HeaderValue::from_static("keep-alive"),
        );
        Self {
            endpoint,
            http_client,
            header_map,
            semaphore,
            config,
            in_flight_requests,
        }
    }

    async fn _execute<'a>(
        &self,
        execution_request: HttpExecutionRequest<'a>,
    ) -> Result<SharedResponse, SubgraphExecutorError> {
        // We may want to remove it, but let's see.
        let mut body = BytesMut::with_capacity(4096);
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
        body.put(CLOSE_BRACE);

        let mut req = hyper::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .version(Version::HTTP_11)
            .body(Full::new(body.freeze()))
            .map_err(|e| {
                SubgraphExecutorError::RequestBuildFailure(self.endpoint.to_string(), e.to_string())
            })?;

        *req.headers_mut() = self.header_map.clone();

        let res = self.http_client.request(req).await.map_err(|e| {
            SubgraphExecutorError::RequestFailure(self.endpoint.to_string(), e.to_string())
        })?;

        let (parts, body) = res.into_parts();

        Ok(SharedResponse {
            status: parts.status,
            body: body
                .collect()
                .await
                .map_err(|e| {
                    SubgraphExecutorError::RequestFailure(self.endpoint.to_string(), e.to_string())
                })?
                .to_bytes(),
            headers: parts.headers,
        })
    }

    fn error_to_graphql_bytes(&self, e: SubgraphExecutorError) -> Bytes {
        let graphql_error: GraphQLError = format!(
            "Failed to execute request to subgraph {}: {}",
            self.endpoint, e
        )
        .into();
        let errors = vec![graphql_error];
        // This unwrap is safe as GraphQLError serialization shouldn't fail.
        let errors_bytes = sonic_rs::to_vec(&errors).unwrap();
        let mut buffer = BytesMut::new();
        buffer.put_slice(b"{\"errors\":");
        buffer.put_slice(&errors_bytes);
        buffer.put_slice(b"}");
        buffer.freeze()
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    async fn execute<'a>(&self, execution_request: HttpExecutionRequest<'a>) -> Bytes {
        if !self.config.dedupe_enabled || !execution_request.dedupe {
            // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
            // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
            let _permit = self.semaphore.acquire().await.unwrap();
            return match self._execute(execution_request).await {
                Ok(shared_response) => shared_response.body,
                Err(e) => self.error_to_graphql_bytes(e),
            };
        }

        let fingerprint = RequestFingerprint::new(
            &http::Method::POST,
            &self.endpoint,
            &self.header_map,
            execution_request
                .representations
                .as_ref()
                .map_or(&[], |b| b.as_ref()),
            &self.config.dedupe_fingerprint_headers,
        );

        // Clone the cell from the map, dropping the lock from the DashMap immediately.
        // Prevents any deadlocks.
        let cell = self
            .in_flight_requests
            .entry(fingerprint.clone())
            .or_default()
            .value()
            .clone();

        let response_result = cell
            .get_or_try_init(|| async {
                let res = {
                    // This unwrap is safe because the semaphore is never closed during the application's lifecycle.
                    // `acquire()` only fails if the semaphore is closed, so this will always return `Ok`.
                    let _permit = self.semaphore.acquire().await.unwrap();
                    self._execute(execution_request).await
                };
                // It's important to remove the entry from the map before returning the result.
                // This ensures that once the OnceCell is set, no future requests can join it.
                // The cache is for the lifetime of the in-flight request only.
                self.in_flight_requests.remove(&fingerprint);
                res
            })
            .await;

        match response_result {
            Ok(shared_response) => shared_response.body.clone(),
            Err(e) => self.error_to_graphql_bytes(e.clone()),
        }
    }
}
