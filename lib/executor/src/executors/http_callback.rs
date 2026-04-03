use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use http::{HeaderMap, HeaderValue};
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Version;
use tracing::{debug, error, trace};
use ulid::Ulid;

use crate::executors::active_subscriptions::{ActiveSubscriptions, BroadcastItem, CallbackState};
use crate::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use crate::executors::error::SubgraphExecutorError;
use crate::executors::http::{build_request_body, HttpClient};
use crate::plugin_context::PluginRequestState;
use crate::response::subgraph_response::SubgraphResponse;

pub const CALLBACK_PROTOCOL_VERSION: &str = "callback/1.0";
pub const SUBSCRIPTION_PROTOCOL_HEADER: &str = "subscription-protocol";

pub struct HttpCallbackSubgraphExecutor {
    pub subgraph_name: String,
    pub endpoint: http::Uri,
    pub http_client: Arc<HttpClient>,
    pub header_map: HeaderMap,
    pub callback_base_url: String,
    pub heartbeat_interval_ms: u64,
    pub active_subscriptions: ActiveSubscriptions,
}

impl HttpCallbackSubgraphExecutor {
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        http_client: Arc<HttpClient>,
        callback_base_url: String,
        heartbeat_interval_ms: u64,
        active_subscriptions: ActiveSubscriptions,
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
        execution_request: &mut SubgraphExecutionRequest<'_>,
        subscription_id: &str,
        verifier: &str,
    ) -> Result<Vec<u8>, SubgraphExecutorError> {
        let callback_url = format!(
            "{}/{}",
            self.callback_base_url.trim_end_matches('/'),
            subscription_id
        );
        let extensions = execution_request.extensions.get_or_insert_default();

        let subscription_ext = sonic_rs::json!({
            "callbackUrl": callback_url,
            "subscriptionId": subscription_id,
            "verifier": verifier,
            "heartbeatIntervalMs": self.heartbeat_interval_ms
        });
        extensions.insert("subscription".to_string(), subscription_ext);

        build_request_body(execution_request)
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
        _plugin_req_state: Option<&'a PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'a>, SubgraphExecutorError> {
        Err(SubgraphExecutorError::HttpCallbackNoSingle)
    }

    #[tracing::instrument(level = "trace", skip_all, fields(subgraph_name = %self.subgraph_name))]
    async fn subscribe<'a>(
        &self,
        mut execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> Result<
        BoxStream<'static, Result<SubgraphResponse<'static>, SubgraphExecutorError>>,
        SubgraphExecutorError,
    > {
        let verifier = Ulid::new().to_string();

        let callback_state = CallbackState {
            verifier: verifier.clone(),
            // initialize last_heartbeat to now + heartbeat_interval so the enforcer
            // won't evict the subscription before the subgraph's initial check arrives.
            // the initial check from the subgraph can take up to heartbeat_interval to
            // arrive (due to network latency), and without this head start the enforcer
            // would evict the subscription before the first heartbeat is recorded.
            last_heartbeat: Arc::new(Mutex::new(
                Instant::now() + Duration::from_millis(self.heartbeat_interval_ms),
            )),
        };

        let (handle, _sender, mut receiver, guard) = self
            .active_subscriptions
            .register(None, Some(callback_state));

        let subscription_id = handle.id().to_string();

        let body = self.build_request_body(&mut execution_request, &subscription_id, &verifier)?;

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
            // hold the handle and guard so the subscription entry is removed when the stream ends
            let _handle = handle;
            let _guard = guard;

            trace!(subscription_id = %subscription_id, "HTTP callback subscription stream started");

            loop {
                match receiver.recv().await {
                    Ok(BroadcastItem::Event(payload)) => {
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
                    Ok(BroadcastItem::Error(errors)) => {
                        trace!(subscription_id = %subscription_id, "received close with error");
                        if !errors.is_empty() {
                            yield Ok(SubgraphResponse {
                                errors: Some(errors),
                                ..Default::default()
                            });
                        }
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // slow consumer, skip missed messages and continue
                        trace!(subscription_id = %subscription_id, lagged = n, "broadcast receiver lagged, skipping missed messages");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        trace!(subscription_id = %subscription_id, "broadcast channel closed");
                        break;
                    }
                }
            }

            trace!(subscription_id = %subscription_id, "HTTP callback subscription stream ended");
        }))
    }
}
