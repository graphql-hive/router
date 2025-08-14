use std::sync::Arc;

use async_trait::async_trait;
use bytes::BufMut;
use bytes::BytesMut;
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
}

const FIRST_VARIABLE_STR: &[u8] = b",\"variables\":{";
const FIRST_QUOTE_STR: &[u8] = b"{\"query\":";

impl HTTPSubgraphExecutor {
    pub fn new(
        endpoint: http::Uri,
        http_client: Arc<Client<HttpConnector, Full<Bytes>>>,
        semaphore: Arc<Semaphore>,
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
        }
    }

    async fn _execute<'a>(
        &self,
        execution_request: HttpExecutionRequest<'a>,
    ) -> Result<Bytes, SubgraphExecutorError> {
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

        Ok(res
            .into_body()
            .collect()
            .await
            .map_err(|e| {
                SubgraphExecutorError::RequestFailure(self.endpoint.to_string(), e.to_string())
            })?
            .to_bytes())
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    async fn execute<'a>(&self, execution_request: HttpExecutionRequest<'a>) -> Bytes {
        // This unwrap is safe because the semaphore is never closed during the gateway lifecycle.
        // The acquire() only fails if the semaphore is closed, so this will always return Ok.
        let _permit = self.semaphore.acquire().await.unwrap();

        match self._execute(execution_request).await {
            Ok(bytes) => bytes,
            Err(e) => {
                let graphql_error: GraphQLError = format!(
                    "Failed to execute request to subgraph {}: {}",
                    self.endpoint, e
                )
                .into();
                let errors = vec![graphql_error];
                let errors_bytes = sonic_rs::to_vec(&errors).unwrap();
                let mut buffer = BytesMut::new();
                buffer.put_slice(b"{\"errors\":");
                buffer.put_slice(&errors_bytes);
                buffer.put_slice(b"}");
                buffer.freeze()
            }
        }
    }
}
