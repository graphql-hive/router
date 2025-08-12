use std::sync::Arc;

use async_trait::async_trait;
use bytes::{BufMut, Bytes, BytesMut};
use cyper::Client;
use http::HeaderMap;
use http::HeaderValue;
use url::Url;

use crate::executors::common::HttpExecutionRequest;
use crate::executors::common::SubgraphExecutor;
use crate::executors::error::SubgraphExecutorError;
use crate::json_writer::BytesMutExt;
use crate::response::graphql_error::GraphQLError;
use crate::utils::consts::CLOSE_BRACE;
use crate::utils::consts::COLON;
use crate::utils::consts::COMMA;
use crate::utils::consts::QUOTE;

#[derive(Debug)]
pub struct HTTPSubgraphExecutor {
    pub endpoint: Url,
    pub http_client: Arc<Client>,
    pub header_map: HeaderMap,
}

const FIRST_VARIABLE_STR: &[u8] = b",\"variables\":{";
const FIRST_QUOTE_STR: &[u8] = b"{\"query\":";

impl HTTPSubgraphExecutor {
    pub fn new(endpoint: &str, http_client: Arc<Client>) -> Result<Self, SubgraphExecutorError> {
        let endpoint = endpoint.parse::<Url>().map_err(|e| {
            SubgraphExecutorError::EndpointParseFailure(endpoint.to_string(), e.to_string())
        })?;
        let mut header_map = HeaderMap::new();
        header_map.insert(
            "Content-Type",
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        Ok(HTTPSubgraphExecutor {
            endpoint,
            http_client,
            header_map,
        })
    }

    async fn _execute<'a>(
        &self,
        execution_request: HttpExecutionRequest<'a>,
    ) -> Result<Bytes, SubgraphExecutorError> {
        // We may want to remove it, but let's see.
        let mut body = BytesMut::with_capacity(4096);
        body.put(FIRST_QUOTE_STR);
        body.write_and_escape_string(execution_request.query);
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

        let res = self
            .http_client
            .post(self.endpoint.clone())
            .map_err(|e| {
                SubgraphExecutorError::RequestBuildFailure(self.endpoint.to_string(), e.to_string())
            })?
            .body(body.freeze())
            .headers(self.header_map.clone())
            .send()
            .await
            .map_err(|e| {
                SubgraphExecutorError::RequestBuildFailure(self.endpoint.to_string(), e.to_string())
            })?;

        let bytes = res.bytes().await.map_err(|e| {
            SubgraphExecutorError::RequestFailure(self.endpoint.to_string(), e.to_string())
        })?;

        Ok(bytes)
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    async fn execute<'a>(&self, execution_request: HttpExecutionRequest<'a>) -> Bytes {
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
