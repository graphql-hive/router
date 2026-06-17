use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::stream::BoxStream;
use futures_util::StreamExt;
use hive_router_internal::telemetry::TelemetryContext;
use ntex::rt;
use tokio::sync::Notify;
use tracing::debug;

use crate::executors::common::{SubgraphExecutionRequest, SubgraphExecutor};
use crate::executors::error::SubgraphExecutorError;
use crate::executors::graphql_transport_ws::build_subscribe_payload;
use crate::executors::websocket_client::{connect, WsClient};
use crate::response::subgraph_response::SubgraphResponse;

/// A single-slot, drop-oldest channel bridging the subgraph reader task (which runs on the
/// non-Send ntex runtime) to the Send query-plan execution stream that consumes it.
///
/// The reader always overwrites the slot with the latest event. When the consumer falls behind -
/// e.g. per-event query planning can't keep up with a fast subgraph - the stale event is dropped
/// and the consumer resumes from the freshest one. The subscription is never terminated for being
/// slow. Downstream, the shared broadcast applies the same drop-oldest policy across fan-out
/// clients.
struct LatestSlot<T> {
    value: Mutex<Option<T>>,
    notify: Notify,
    closed: AtomicBool,
}

impl<T> LatestSlot<T> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(None),
            notify: Notify::new(),
            closed: AtomicBool::new(false),
        })
    }

    /// Store the latest value. Returns `true` if it replaced a previous unconsumed value - i.e.
    /// the consumer fell behind and that older event was dropped.
    fn store(&self, value: T) -> bool {
        let dropped = self.value.lock().unwrap().replace(value).is_some();
        self.notify.notify_one();
        dropped
    }

    /// Signal that no more values will be produced. The consumer stops once the slot is drained.
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.notify.notify_one();
    }

    /// Await the freshest available value, or `None` once the producer has closed and the slot is
    /// drained.
    async fn next(&self) -> Option<T> {
        loop {
            // register for notification before reading, so a concurrent store/close happening
            // between the read and the await is not missed
            let notified = self.notify.notified();
            if let Some(value) = self.value.lock().unwrap().take() {
                return Some(value);
            }
            if self.closed.load(Ordering::Acquire) {
                // a value may have landed between the take above and this check
                return self.value.lock().unwrap().take();
            }
            notified.await;
        }
    }
}

pub struct WsSubgraphExecutor {
    subgraph_name: String,
    endpoint: http::Uri,
    tls_config: Option<Arc<rustls::ClientConfig>>,
    telemetry_context: Arc<TelemetryContext>,
}

impl WsSubgraphExecutor {
    pub fn new(
        subgraph_name: String,
        endpoint: http::Uri,
        tls_config: Option<Arc<rustls::ClientConfig>>,
        telemetry_context: Arc<TelemetryContext>,
    ) -> Self {
        Self {
            subgraph_name,
            endpoint,
            tls_config,
            telemetry_context,
        }
    }
}

#[async_trait]
impl SubgraphExecutor for WsSubgraphExecutor {
    fn endpoint(&self) -> &http::Uri {
        &self.endpoint
    }

    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
        _plugin_req_state: Option<&'a crate::plugin_context::PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        let tls_config = self.tls_config.clone();
        let custom_scalar_paths = execution_request.custom_scalar_paths.cloned();
        debug!(
            "establishing WebSocket connection to subgraph {} at {}",
            subgraph_name, endpoint
        );

        let (subscribe_payload, init_payload) = build_subscribe_payload(execution_request);

        let (tx, rx) = oneshot::channel();

        // run this on ntex runtime instead of Handle::spawn because the websocket path builds
        // and awaits futures that capture ntex local types like Rc and RefCell via WsClient.
        // those futures are not Send, so they cannot cross a tokio multi-threaded spawn boundary.
        // ntex::rt::spawn keeps the whole websocket flow on the local ntex runtime, while this
        // async_trait method still stays Send by awaiting only the futures oneshot receiver here.
        // this task ends after the first websocket response is forwarded through the oneshot,
        // or earlier if connect/init fails.
        rt::spawn(async move {
            let result = async {
                let connection = match connect(&endpoint, tls_config).await {
                    Ok(conn) => conn,
                    Err(e) => {
                        return Err(SubgraphExecutorError::WebSocketConnectFailure(
                            endpoint.to_string(),
                            e.to_string(),
                        ));
                    }
                };

                let mut client = match WsClient::init(connection, init_payload).await {
                    Ok(client) => client,
                    Err(e) => {
                        return Err(SubgraphExecutorError::WebSocketHandshakeFailure(
                            endpoint.to_string(),
                            e.to_string(),
                        ));
                    }
                };

                debug!(
                    "WebSocket connection to subgraph {} at {} established",
                    subgraph_name, endpoint
                );

                let mut stream = client
                    .subscribe(subscribe_payload, custom_scalar_paths)
                    .await;

                match stream.next().await {
                    Some(response) => Ok(response),
                    None => Err(SubgraphExecutorError::WebSocketStreamClosedEmpty(
                        endpoint.to_string(),
                    )),
                }
            }
            .await;

            let _ = tx.send(result);
        });

        rx.await
            .map_err(|_| SubgraphExecutorError::WebSocketArbiterChannelClosed)?
    }

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
    ) -> Result<
        BoxStream<'static, Result<SubgraphResponse<'static>, SubgraphExecutorError>>,
        SubgraphExecutorError,
    > {
        // Bridge the non-Send subgraph reader task to the Send query-plan execution stream with a
        // drop-oldest slot: if per-event planning falls behind a fast subgraph, stale events are
        // dropped and we resume from the freshest one, so a slow consumer never terminates the
        // subscription. Downstream, the shared broadcast applies the same drop-oldest policy
        // across fan-out clients.
        let slot = LatestSlot::<Result<SubgraphResponse<'static>, SubgraphExecutorError>>::new();
        let writer = slot.clone();

        let endpoint = self.endpoint.clone();
        let subgraph_name = self.subgraph_name.clone();
        let tls_config = self.tls_config.clone();
        let telemetry_context = self.telemetry_context.clone();
        let custom_scalar_paths = execution_request.custom_scalar_paths.cloned();

        let (subscribe_payload, init_payload) = build_subscribe_payload(execution_request);

        debug!(
            "establishing WebSocket subscription connection to subgraph {} at {}",
            self.subgraph_name, self.endpoint
        );

        // no await intentionally. the task runs the subscription in the background and forwards
        // responses through the slot. The spawned future itself stays local to the ntex runtime,
        // so it can hold non-Send websocket client state. this task ends when the websocket stream
        // completes or the connection fails; on exit it closes the slot so the consumer stops.
        drop(rt::spawn(async move {
            async {
                let connection = match connect(&endpoint, tls_config).await {
                    Ok(conn) => conn,
                    Err(e) => {
                        writer.store(Err(SubgraphExecutorError::WebSocketConnectFailure(
                            endpoint.to_string(),
                            e.to_string(),
                        )));
                        return;
                    }
                };

                let mut client = match WsClient::init(connection, init_payload).await {
                    Ok(client) => client,
                    Err(e) => {
                        writer.store(Err(SubgraphExecutorError::WebSocketHandshakeFailure(
                            endpoint.to_string(),
                            e.to_string(),
                        )));
                        return;
                    }
                };

                debug!(
                    "WebSocket subscription connection to subgraph {} at {} established",
                    subgraph_name, endpoint
                );

                let mut stream = client
                    .subscribe(subscribe_payload, custom_scalar_paths)
                    .await;

                while let Some(response) = stream.next().await {
                    if writer.store(Ok(response)) {
                        // the consumer fell behind and a stale event was dropped
                        telemetry_context
                            .metrics
                            .subscription
                            .record_dropped_event(&subgraph_name);
                    }
                }
            }
            .await;

            // producer finished (stream ended or failed) - let the consumer drain and stop
            writer.close();
        }));

        let reader = slot;
        Ok(Box::pin(async_stream::stream! {
            while let Some(response) = reader.next().await {
                yield response;
            }
        }))
    }
}
