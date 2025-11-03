use std::collections::BTreeMap;
use std::sync::Arc;

use crate::executors::common::HttpExecutionResponse;
use crate::executors::dedupe::{request_fingerprint, ABuildHasher, SharedResponse};
use crate::utils::expression::execute_expression_with_value;
use dashmap::DashMap;
use hive_router_config::HiveRouterConfig;
use tokio::sync::OnceCell;

use async_trait::async_trait;

use bytes::{BufMut, Bytes, BytesMut};
use hmac::{Hmac, Mac};
use http::HeaderMap;
use http::HeaderValue;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Version;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use sha2::Sha256;
use tokio::sync::Semaphore;
use tracing::debug;
use vrl::compiler::Program as VrlProgram;

use crate::executors::common::HttpExecutionRequest;
use crate::executors::error::SubgraphExecutorError;
use crate::response::graphql_error::GraphQLError;
use crate::utils::consts::CLOSE_BRACE;
use crate::utils::consts::COLON;
use crate::utils::consts::COMMA;
use crate::utils::consts::QUOTE;
use crate::{executors::common::SubgraphExecutor, json_writer::write_and_escape_string};
use vrl::core::Value as VrlValue;

#[derive(Debug)]
pub struct HTTPSubgraphExecutor {
    pub subgraph_name: String,
    pub endpoint: http::Uri,
    pub http_client: Arc<Client<HttpsConnector<HttpConnector>, Full<Bytes>>>,
    pub header_map: HeaderMap,
    pub semaphore: Arc<Semaphore>,
    pub config: Arc<HiveRouterConfig>,
    pub in_flight_requests: Arc<DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>>,
    pub should_sign_hmac: BooleanOrProgram,
}

const FIRST_VARIABLE_STR: &[u8] = b",\"variables\":{";
const FIRST_QUOTE_STR: &[u8] = b"{\"query\":";
const FIRST_EXTENSION_STR: &[u8] = b",\"extensions\":{";

pub type HttpClient = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug)]
pub enum BooleanOrProgram {
    Boolean(bool),
    Program(Box<VrlProgram>),
}

impl HTTPSubgraphExecutor {
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        http_client: Arc<HttpClient>,
        semaphore: Arc<Semaphore>,
        config: Arc<HiveRouterConfig>,
        in_flight_requests: Arc<DashMap<u64, Arc<OnceCell<SharedResponse>>, ABuildHasher>>,
        should_sign_hmac: BooleanOrProgram,
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
            config,
            in_flight_requests,
            should_sign_hmac,
        }
    }

    fn build_request_body<'exec, 'req>(
        &self,
        execution_request: &HttpExecutionRequest<'exec, 'req>,
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

        let should_sign_hmac = match &self.should_sign_hmac {
            BooleanOrProgram::Boolean(b) => *b,
            BooleanOrProgram::Program(expr) => {
                // .subgraph
                let subgraph_value = VrlValue::Object(BTreeMap::from([(
                    "name".into(),
                    VrlValue::Bytes(Bytes::from(self.subgraph_name.to_owned())),
                )]));
                // .request
                let request_value: VrlValue = execution_request.client_request.into();
                let target_value = VrlValue::Object(BTreeMap::from([
                    ("subgraph".into(), subgraph_value),
                    ("request".into(), request_value),
                ]));
                let result = execute_expression_with_value(expr, target_value);
                match result {
                    Ok(VrlValue::Boolean(b)) => b,
                    Ok(_) => {
                        return Err(SubgraphExecutorError::HMACSignatureError(
                            "HMAC signature expression did not evaluate to a boolean".to_string(),
                        ));
                    }
                    Err(e) => {
                        return Err(SubgraphExecutorError::HMACSignatureError(format!(
                            "HMAC signature expression evaluation error: {}",
                            e
                        )));
                    }
                }
            }
        };

        let hmac_signature_ext = if should_sign_hmac {
            let mut mac = HmacSha256::new_from_slice(self.config.hmac_signature.secret.as_bytes())
                .map_err(|e| {
                    SubgraphExecutorError::HMACSignatureError(format!(
                        "Failed to create HMAC instance: {}",
                        e
                    ))
                })?;
            let mut body_without_extensions = body.clone();
            body_without_extensions.put(CLOSE_BRACE);
            mac.update(&body_without_extensions);
            let result = mac.finalize();
            let result_bytes = result.into_bytes();
            Some(result_bytes)
        } else {
            None
        };

        let mut first_extension = true;

        if let Some(hmac_bytes) = hmac_signature_ext {
            if first_extension {
                body.put(FIRST_EXTENSION_STR);
                first_extension = false;
            } else {
                body.put(COMMA);
            }
            body.put(QUOTE);
            body.put(self.config.hmac_signature.extension_name.as_bytes());
            body.put(QUOTE);
            body.put(COLON);
            let hmac_hex = hex::encode(hmac_bytes);
            body.put(QUOTE);
            body.put(hmac_hex.as_bytes());
            body.put(QUOTE);
        }

        if let Some(extensions) = &execution_request.extensions {
            for (extension_name, extension_value) in extensions {
                if first_extension {
                    body.put(FIRST_EXTENSION_STR);
                    first_extension = false;
                } else {
                    body.put(COMMA);
                }
                body.put(QUOTE);
                body.put(extension_name.as_bytes());
                body.put(QUOTE);
                body.put(COLON);
                let value_str = sonic_rs::to_string(extension_value).map_err(|err| {
                    SubgraphExecutorError::ExtensionSerializationFailure(
                        extension_name.to_string(),
                        err.to_string(),
                    )
                })?;
                body.put(value_str.as_bytes());
            }
        }

        if !first_extension {
            body.put(CLOSE_BRACE);
        }

        body.put(CLOSE_BRACE);

        println!("Built request body: {}", String::from_utf8_lossy(&body));
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
    #[tracing::instrument(skip_all, fields(subgraph_name = self.subgraph_name))]
    async fn execute<'exec, 'req>(
        &self,
        execution_request: HttpExecutionRequest<'exec, 'req>,
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
