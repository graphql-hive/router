use std::io::Write;
use std::sync::Arc;

use async_trait::async_trait;
use cyper::Client;
use http::HeaderMap;
use http::HeaderValue;
use tracing::{error, instrument, trace};
use url::Url;

use crate::{
    executors::common::SubgraphExecutor, json_writer::write_and_escape_string, ExecutionResult,
    SubgraphExecutionRequest,
};

#[derive(Debug)]
pub struct HTTPSubgraphExecutor {
    pub endpoint: Url,
    pub http_client: Arc<Client>,
    pub header_map: HeaderMap,
}

const FIRST_VARIABLE_STR: &[u8; 14] = b",\"variables\":{";

impl HTTPSubgraphExecutor {
    pub fn new(endpoint: &str, http_client: Arc<Client>) -> Self {
        let endpoint = endpoint
            .parse::<Url>()
            .expect("Failed to parse endpoint as URI");
        let mut header_map = HeaderMap::new();
        header_map.insert(
            "Content-Type",
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        HTTPSubgraphExecutor {
            endpoint,
            http_client,
            header_map,
        }
    }

    fn write_body(
        execution_request: &SubgraphExecutionRequest,
        writer: &mut impl Write,
    ) -> std::io::Result<()> {
        writer.write_all(b"{\"query\":")?;
        write_and_escape_string(writer, execution_request.query)?;

        let mut first_variable = true;
        if let Some(variables) = &execution_request.variables {
            for (variable_name, variable_value) in variables {
                if first_variable {
                    writer.write_all(FIRST_VARIABLE_STR)?;
                    first_variable = false;
                } else {
                    writer.write_all(b",")?;
                }
                writer.write_all(b"\"")?;
                writer.write_all(variable_name.as_bytes())?;
                writer.write_all(b"\":")?;
                serde_json::to_writer(&mut *writer, variable_value)?;
            }
        }
        if let Some(representations) = &execution_request.representations {
            if first_variable {
                writer.write_all(FIRST_VARIABLE_STR)?;
                first_variable = false;
            } else {
                writer.write_all(b",")?;
            }
            writer.write_all(b"\"representations\":")?;
            writer.write_all(representations)?;
        }
        // "first_variable" should be still true if there are no variables
        if !first_variable {
            writer.write_all(b"}")?;
        }
        writer.write_all(b"}")?;
        Ok(())
    }

    async fn _execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
    ) -> Result<ExecutionResult, String> {
        trace!("Executing HTTP request to subgraph at {}", self.endpoint);

        // We may want to remove it, but let's see.
        let mut body = Vec::with_capacity(4096);
        Self::write_body(&execution_request, &mut body)
            .map_err(|e| format!("Failed to write request body: {}", e))?;

        let res = self
            .http_client
            .post(self.endpoint.clone())
            .unwrap()
            .body(body)
            .headers(self.header_map.clone())
            .send()
            .await
            .map_err(|e| {
                format!(
                    "Failed to send request to subgraph {}: {}",
                    self.endpoint, e
                )
            })?;

        let bytes = res.bytes().await.map_err(|e| {
            format!(
                "Failed to parse response from subgraph {}: {}",
                self.endpoint, e
            )
        })?;

        unsafe {
            sonic_rs::from_slice_unchecked(&bytes).map_err(|e| {
                format!(
                    "Failed to parse response from subgraph {}: {}",
                    self.endpoint, e
                )
            })
        }
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    #[instrument(level = "trace", skip(self), name = "http_subgraph_execute", fields(endpoint = %self.endpoint))]
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
    ) -> ExecutionResult {
        self._execute(execution_request).await.unwrap_or_else(|e| {
            error!(e);
            ExecutionResult::from_error_message(e)
        })
    }
}
