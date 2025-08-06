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

use crate::executors::common::HttpExecutionRequest;
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
}

const FIRST_VARIABLE_STR: &[u8] = b",\"variables\":{";
const FIRST_QUOTE_STR: &[u8] = b"{\"query\":";

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
        execution_request: HttpExecutionRequest<'a>,
    ) -> Result<Bytes, String> {
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
                    format!("Failed to serialize variable '{}': {}", variable_name, err)
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

        Ok(res
            .into_body()
            .collect()
            .await
            .map_err(|e| {
                format!(
                    "Failed to parse response from subgraph {}: {}",
                    self.endpoint, e
                )
            })?
            .to_bytes())
    }
}

#[async_trait]
impl SubgraphExecutor for HTTPSubgraphExecutor {
    async fn execute<'a>(&self, execution_request: HttpExecutionRequest<'a>) -> Bytes {
        self._execute(execution_request).await.unwrap_or_else(|e| {
            panic!(
                "Failed to execute request to subgraph {}: {}",
                self.endpoint, e
            );
        })
    }
}
