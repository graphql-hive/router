use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::{BufMut, Bytes};
use dashmap::DashMap;
use futures::stream::BoxStream;
use http::{HeaderMap, HeaderValue};
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Version;
use tokio::sync::mpsc;
use tracing::{debug, error, trace};
use uuid::Uuid;

use crate::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use crate::executors::error::SubgraphExecutorError;
use crate::executors::http::HttpClient;
use crate::json_writer::write_and_escape_string;
use crate::plugin_context::PluginRequestState;
use crate::response::graphql_error::GraphQLError;
use crate::response::subgraph_response::SubgraphResponse;
use crate::utils::consts::{CLOSE_BRACE, COLON, COMMA, QUOTE};

pub const CALLBACK_PROTOCOL_VERSION: &str = "callback/1.0";
pub const SUBSCRIPTION_PROTOCOL_HEADER: &str = "subscription-protocol";

type SubscriptionId = String;

#[derive(Clone)]
pub struct ActiveSubscription {
    pub verifier: String,
    pub sender: mpsc::UnboundedSender<CallbackMessage>,
    pub last_heartbeat: Arc<Mutex<Instant>>,
}

impl ActiveSubscription {
    pub fn record_heartbeat(&self) {
        *self.last_heartbeat.lock().unwrap() = Instant::now();
    }
}

#[derive(Debug)]
pub enum CallbackMessage {
    Next { payload: Bytes },
    Complete { errors: Option<Vec<GraphQLError>> },
}

pub type ActiveSubscriptionsMap = Arc<DashMap<SubscriptionId, ActiveSubscription>>;

struct SubscriptionGuard {
    subscription_id: SubscriptionId,
    active_subscriptions: ActiveSubscriptionsMap,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        self.active_subscriptions.remove(&self.subscription_id);
        trace!(subscription_id = %self.subscription_id, "HTTP callback subscription entry removed from active subscriptions");
    }
}

pub struct HttpCallbackSubgraphExecutor {
    pub subgraph_name: String,
    pub endpoint: http::Uri,
    pub http_client: Arc<HttpClient>,
    pub header_map: HeaderMap,
    pub callback_base_url: String,
    pub heartbeat_interval_ms: u64,
    pub active_subscriptions: ActiveSubscriptionsMap,
}

const FIRST_QUOTE_STR: &[u8] = b"{\"query\":";
const FIRST_VARIABLE_STR: &[u8] = b",\"variables\":{";

impl HttpCallbackSubgraphExecutor {
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        http_client: Arc<HttpClient>,
        callback_base_url: String,
        heartbeat_interval_ms: u64,
        active_subscriptions: ActiveSubscriptionsMap,
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
            http::header::ACCEPT,
            HeaderValue::from_static("application/json;callbackSpec=1.0"),
        );

        Self {
            subgraph_name,
            endpoint,
            http_client,
            header_map,
            callback_base_url,
            heartbeat_interval_ms,
            active_subscriptions,
        }
    }

    fn build_request_body(
        &self,
        execution_request: &SubgraphExecutionRequest<'_>,
        subscription_id: &str,
        verifier: &str,
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
                        err,
                    )
                })?;
                body.put(value_str.as_bytes());
            }
        }

        if let Some(raw_variable_values) = &execution_request.raw_variable_values {
            for (variable_name, variable_value) in raw_variable_values {
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
                body.extend_from_slice(variable_value);
            }
        }

        if !first_variable {
            body.put(CLOSE_BRACE);
        }

        // Build extensions with subscription callback info
        let callback_url = format!(
            "{}/{}",
            self.callback_base_url.trim_end_matches('/'),
            subscription_id
        );
        let mut extensions: HashMap<String, sonic_rs::Value> =
            execution_request.extensions.clone().unwrap_or_default();

        let subscription_ext = sonic_rs::json!({
            "callbackUrl": callback_url,
            "subscriptionId": subscription_id,
            "verifier": verifier,
            "heartbeatIntervalMs": self.heartbeat_interval_ms
        });
        extensions.insert("subscription".to_string(), subscription_ext);

        let extensions_str = sonic_rs::to_string(&extensions).map_err(|err| {
            SubgraphExecutorError::VariablesSerializationFailure("extensions".to_string(), err)
        })?;

        body.put(COMMA);
        body.put("\"extensions\":".as_bytes());
        body.extend_from_slice(extensions_str.as_bytes());

        body.put(CLOSE_BRACE);

        Ok(body)
    }
}

#[async_trait]
impl SubgraphExecutor for HttpCallbackSubgraphExecutor {
    fn endpoint(&self) -> &http::Uri {
        &self.endpoint
    }

    #[tracing::instrument(level = "trace", skip_all, fields(subgraph_name = %self.subgraph_name))]
    async fn execute<'a>(
        &self,
        _execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
        _plugin_req_state: &'a Option<PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
        Err(SubgraphExecutorError::HttpCallbackNoSingle)
    }

    #[tracing::instrument(level = "trace", skip_all, fields(subgraph_name = %self.subgraph_name))]
    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> Result<BoxStream<'static, Result<SubgraphResponse<'static>, SubgraphExecutorError>>, SubgraphExecutorError> {
        let subscription_id = Uuid::new_v4().to_string();
        let verifier = Uuid::new_v4().to_string();

        let body = self.build_request_body(&execution_request, &subscription_id, &verifier)?;

        let (tx, mut rx) = mpsc::unbounded_channel::<CallbackMessage>();
        self.active_subscriptions.insert(
            subscription_id.clone(),
            ActiveSubscription {
                verifier,
                sender: tx,
                // initialize last_heartbeat to now + heartbeat_interval so the enforcer
                // won't evict the subscription before the subgraph's initial check arrives.
                // the initial check from the subgraph can take up to heartbeat_interval to
                // arrive (due to network latency), and without this head start the enforcer
                // would evict the subscription before the first heartbeat is recorded.
                last_heartbeat: Arc::new(Mutex::new(
                    Instant::now() + Duration::from_millis(self.heartbeat_interval_ms),
                )),
            },
        );

        // guard removes the entry from `active_subscriptions` when dropped
        let guard = SubscriptionGuard {
            subscription_id: subscription_id.clone(),
            active_subscriptions: self.active_subscriptions.clone(),
        };

        let mut req = hyper::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .version(Version::HTTP_11)
            .body(Full::new(Bytes::from(body)))
            .map_err(SubgraphExecutorError::RequestBuildFailure)?;

        let mut headers = execution_request.headers;
        self.header_map.iter().for_each(|(key, value)| {
            headers.insert(key, value.clone());
        });
        *req.headers_mut() = headers;

        debug!(
            subscription_id = %subscription_id,
            "sending HTTP callback subscription request to subgraph {} at {}",
            self.subgraph_name,
            self.endpoint.to_string()
        );

        let req_fut = self.http_client.request(req);
        let res = if let Some(timeout_duration) = timeout {
            tokio::time::timeout(timeout_duration, req_fut)
                .await
                .map_err(|_| {
                    SubgraphExecutorError::RequestTimeout(
                        self.endpoint.to_string(),
                        timeout_duration.as_millis(),
                    )
                })?
                .map_err(SubgraphExecutorError::RequestFailure)?
        } else {
            req_fut
                .await
                .map_err(SubgraphExecutorError::RequestFailure)?
        };

        debug!(
            subscription_id = %subscription_id,
            "HTTP callback subscription request to {} completed, status: {}",
            self.endpoint.to_string(),
            res.status()
        );

        if !res.status().is_success() {
            let status = res.status();
            let (_, body) = res.into_parts();
            let body_bytes = body.collect().await.ok().map(|b| b.to_bytes());
            let body_str = body_bytes
                .as_ref()
                .and_then(|b| std::str::from_utf8(b).ok())
                .unwrap_or("(no body)");
            error!(
                subscription_id = %subscription_id,
                status = %status,
                body = body_str,
                "HTTP callback subscription request failed with non-success status"
            );
            return Err(SubgraphExecutorError::HttpCallbackStatusCodeNotOk(status));
        }

        Ok(Box::pin(async_stream::stream! {
            // `guard` is held here; dropping the stream drops `guard`, removing the map entry.
            let _guard = guard;

            trace!(subscription_id = %subscription_id, "HTTP callback subscription stream started");

            while let Some(msg) = rx.recv().await {
                match msg {
                    CallbackMessage::Next { payload } => {
                        trace!(subscription_id = %subscription_id, "received next payload");
                        match SubgraphResponse::deserialize_from_bytes(payload) {
                            Ok(response) => yield Ok(response),
                            Err(e) => {
                                error!(
                                    subscription_id = %subscription_id,
                                    error = %e,
                                    "failed to deserialize callback payload"
                                );
                                yield Err(e);
                                break;
                            }
                        }
                    }
                    CallbackMessage::Complete { errors } => {
                        trace!(subscription_id = %subscription_id, "received complete");
                        if let Some(errors) = errors {
                            if !errors.is_empty() {
                                yield Ok(SubgraphResponse {
                                    errors: Some(errors),
                                    ..Default::default()
                                });
                            }
                        }
                        break;
                    }
                }
            }

            trace!(subscription_id = %subscription_id, "HTTP callback subscription stream ended");
        }))
    }
}
