use std::sync::Arc;

use async_trait::async_trait;
use http::HeaderMap;
use http::HeaderValue;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{body::Bytes, Version};
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use tracing::{error, instrument, trace};

use crate::{
    executors::common::SubgraphExecutor, json_writer::write_and_escape_string, ExecutionResult,
    SubgraphExecutionRequest,
};

#[derive(Debug)]
pub struct HTTPSubgraphExecutor {
    pub endpoint: http::Uri,
    pub http_client: Arc<Client<HttpConnector, Full<Bytes>>>,
    pub header_map: HeaderMap,
}

const FIRST_VARIABLE_STR: &str = ",\"variables\":{";

impl HTTPSubgraphExecutor {
    pub fn new(endpoint: &str, http_client: Arc<Client<HttpConnector, Full<Bytes>>>) -> Self {
        let endpoint = endpoint
            .parse::<http::Uri>()
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

    async fn _execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
    ) -> Result<ExecutionResult, String> {
        trace!("Executing HTTP request to subgraph at {}", self.endpoint);

        // We may want to remove it, but let's see.
        let mut body = String::with_capacity(4096);
        body.push_str("{\"query\":");
        write_and_escape_string(&mut body, execution_request.query);
        let mut first_variable = true;
        if let Some(variables) = &execution_request.variables {
            for (variable_name, variable_value) in variables {
                if first_variable {
                    body.push_str(FIRST_VARIABLE_STR);
                    first_variable = false;
                } else {
                    body.push(',');
                }
                body.push('"');
                body.push_str(variable_name);
                body.push_str("\":");
                let value_str = serde_json::to_string(variable_value).map_err(|err| {
                    format!("Failed to serialize variable '{}': {}", variable_name, err)
                })?;
                body.push_str(&value_str);
            }
        }
        if let Some(representations) = &execution_request.representations {
            if first_variable {
                body.push_str(FIRST_VARIABLE_STR);
                first_variable = false;
            } else {
                body.push(',');
            }
            body.push_str("\"representations\":");
            body.push_str(representations);
        }
        // "first_variable" should be still true if there are no variables
        if !first_variable {
            body.push('}');
        }
        body.push('}');

        let mut req = hyper::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .version(Version::HTTP_11)
            .body(body.into())
            .map_err(|e| {
                format!(
                    "Failed to build request to subgraph {}: {}",
                    self.endpoint, e
                )
            })?;

        *req.headers_mut() = self.header_map.clone();

        let res = self.http_client.request(req).await.map_err(|e| {
            format!(
                "Failed to send request to subgraph {}: {}",
                self.endpoint, e
            )
        })?;

        let bytes = res
            .into_body()
            .collect()
            .await
            .map_err(|e| {
                format!(
                    "Failed to parse response from subgraph {}: {}",
                    self.endpoint, e
                )
            })?
            .to_bytes();

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
