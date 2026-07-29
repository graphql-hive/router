use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use async_trait::async_trait;
use dashmap::{mapref::entry::Entry, DashMap};
use futures::{stream::BoxStream, StreamExt};
use hive_router_internal::{
    telemetry::metrics::subscription_metrics::SubscriptionTransport, telemetry::TelemetryContext,
};
use hive_router_query_planner::planner::plan_nodes::CustomScalarPaths;
use http::{HeaderMap, Uri};
use ntex::rt;
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};

use crate::{
    executors::{
        common::{ConnectionFingerprint, SubgraphExecutionRequest, SubgraphExecutor},
        error::SubgraphExecutorError,
        graphql_transport_ws::SubscribePayload,
        subscription_buffer::drain_into,
        websocket_client::{self, WsClient, WsClientError},
    },
    plugin_context::PluginRequestState,
    response::subgraph_response::SubgraphResponse,
};

type SubscriptionItem = Result<SubgraphResponse<'static>, SubgraphExecutorError>;
type InitResult = Result<Arc<PooledWebSocketExecutor>, PoolInitError>;
type PoolEntries = DashMap<WebSocketConnectionId, PoolEntry>;

/// Identifies one reusable, initialized subgraph WebSocket connection.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WebSocketConnectionId {
    subgraph_name: Arc<str>,
    fingerprint: ConnectionFingerprint,
}

impl WebSocketConnectionId {
    pub fn new(subgraph_name: impl Into<Arc<str>>, fingerprint: ConnectionFingerprint) -> Self {
        Self {
            subgraph_name: subgraph_name.into(),
            fingerprint,
        }
    }
}

// TODO: use thiserror and transparent and whatever to inherit errors #[from]
#[derive(Clone, Debug, thiserror::Error)]
enum PoolInitError {
    #[error("WebSocket connection failed: {0}")]
    Connect(String),
    #[error("WebSocket handshake failed: {0}")]
    Handshake(String),
}

impl PoolInitError {
    fn into_executor_error(self, endpoint: &Uri) -> SubgraphExecutorError {
        match self {
            Self::Connect(error) => {
                SubgraphExecutorError::WebSocketConnectFailure(endpoint.to_string(), error)
            }
            Self::Handshake(error) => {
                SubgraphExecutorError::WebSocketHandshakeFailure(endpoint.to_string(), error)
            }
        }
    }
}

struct ConnectingEntry {
    // each initialization attempt gets a unique token. an attempt can be canceled and replaced
    // while its future or drop guard is still alive, so late cleanup must not remove the newer
    // connecting entry and late success must not overwrite it. Arc::ptr_eq checks exact ownership.
    generation: Arc<()>,
    waiters: Vec<oneshot::Sender<InitResult>>,
}

/// State for one connection identity.
///
/// A successful initialization replaces its exact `Connecting` generation with `Initialized`
/// while the DashMap entry remains occupied. Failed or canceled attempts remove only their own
/// generation, so stale work can never remove or overwrite a successor.
enum PoolEntry {
    Connecting(ConnectingEntry),
    Initialized(Arc<PooledWebSocketExecutor>),
}

pub struct WebSocketInit {
    pub endpoint: Uri,
    pub headers: HeaderMap,
    pub tls_config: Option<Arc<rustls::ClientConfig>>,
    pub buffer_capacity: usize,
    pub idle_timeout: Duration,
    pub telemetry_context: Arc<TelemetryContext>,
}

#[derive(Default)]
pub struct WebSocketPool {
    entries: Arc<PoolEntries>,
}

impl WebSocketPool {
    pub fn get_initialized(
        &self,
        id: &WebSocketConnectionId,
    ) -> Option<Arc<PooledWebSocketExecutor>> {
        self.entries.get(id).and_then(|entry| match entry.value() {
            PoolEntry::Initialized(executor) if !executor.commands.is_closed() => {
                Some(executor.clone())
            }
            PoolEntry::Connecting(_) | PoolEntry::Initialized(_) => None,
        })
    }

    pub async fn get_or_initialize(
        &self,
        id: WebSocketConnectionId,
        init: WebSocketInit,
    ) -> Result<Arc<PooledWebSocketExecutor>, SubgraphExecutorError> {
        let endpoint = init.endpoint.clone();
        let (wait_rx, generation) = match self.entries.entry(id.clone()) {
            Entry::Occupied(mut entry) => match entry.get_mut() {
                PoolEntry::Initialized(executor) if !executor.commands.is_closed() => {
                    return Ok(executor.clone());
                }
                PoolEntry::Connecting(connecting) => {
                    connecting.waiters.retain(|waiter| !waiter.is_closed());
                    let (wait_tx, wait_rx) = oneshot::channel();
                    connecting.waiters.push(wait_tx);
                    (wait_rx, None)
                }
                PoolEntry::Initialized(_) => {
                    // the owner closes commands before eviction. replace a closed entry in place
                    // so callers never join an executor that can no longer accept work.
                    let generation = Arc::new(());
                    let (wait_tx, wait_rx) = oneshot::channel();
                    entry.insert(PoolEntry::Connecting(ConnectingEntry {
                        generation: generation.clone(),
                        waiters: vec![wait_tx],
                    }));
                    (wait_rx, Some(generation))
                }
            },
            Entry::Vacant(entry) => {
                let generation = Arc::new(());
                let (wait_tx, wait_rx) = oneshot::channel();
                entry.insert(PoolEntry::Connecting(ConnectingEntry {
                    generation: generation.clone(),
                    waiters: vec![wait_tx],
                }));
                (wait_rx, Some(generation))
            }
        };

        if let Some(generation) = generation {
            init.telemetry_context
                .metrics
                .subscriptions
                .record_websocket_pool_initialization_started();

            let cleanup = InitializationCleanup::new(self.entries.clone(), id.clone(), generation);
            let telemetry_context = init.telemetry_context.clone();
            rt::spawn(async move {
                let mut cleanup = cleanup;
                match initialize_connection(&cleanup.entries, &cleanup.id, init).await {
                    Ok(connection) => {
                        let executor = connection.executor.clone();
                        let Some(waiters) = cleanup.publish(executor.clone()) else {
                            return;
                        };

                        // publication happens first, so even an immediately exiting owner can only
                        // evict the initialized generation it actually owns.
                        rt::spawn(connection.owner.run());
                        notify_waiters(waiters, Ok(executor));
                    }
                    Err(error) => {
                        telemetry_context
                            .metrics
                            .subscriptions
                            .record_websocket_pool_initialization_failed();
                        let waiters = cleanup.remove();
                        notify_waiters(waiters, Err(error));
                    }
                }
            });
        } else {
            init.telemetry_context
                .metrics
                .subscriptions
                .record_websocket_pool_initialization_joined();
        }

        wait_rx
            .await
            .map_err(|_| SubgraphExecutorError::WebSocketArbiterChannelClosed)?
            .map_err(|error| error.into_executor_error(&endpoint))
    }
}

fn notify_waiters(waiters: Vec<oneshot::Sender<InitResult>>, result: InitResult) {
    for waiter in waiters {
        let _ = waiter.send(result.clone());
    }
}

struct InitializationCleanup {
    entries: Arc<PoolEntries>,
    id: WebSocketConnectionId,
    generation: Arc<()>,
    armed: bool,
}

impl InitializationCleanup {
    fn new(entries: Arc<PoolEntries>, id: WebSocketConnectionId, generation: Arc<()>) -> Self {
        Self {
            entries,
            id,
            generation,
            armed: true,
        }
    }

    fn publish(
        &mut self,
        executor: Arc<PooledWebSocketExecutor>,
    ) -> Option<Vec<oneshot::Sender<InitResult>>> {
        let waiters = match self.entries.entry(self.id.clone()) {
            Entry::Occupied(mut entry) => {
                let PoolEntry::Connecting(connecting) = entry.get_mut() else {
                    return None;
                };
                if !Arc::ptr_eq(&connecting.generation, &self.generation) {
                    return None;
                }

                let waiters = std::mem::take(&mut connecting.waiters);
                entry.insert(PoolEntry::Initialized(executor));
                waiters
            }
            Entry::Vacant(_) => return None,
        };

        self.armed = false;
        Some(waiters)
    }

    fn remove(&mut self) -> Vec<oneshot::Sender<InitResult>> {
        let removed = self.entries.remove_if(&self.id, |_, entry| {
            matches!(
                entry,
                PoolEntry::Connecting(connecting)
                    if Arc::ptr_eq(&connecting.generation, &self.generation)
            )
        });
        self.armed = false;

        match removed {
            Some((_, PoolEntry::Connecting(connecting))) => connecting.waiters,
            Some((_, PoolEntry::Initialized(_))) | None => Vec::new(),
        }
    }
}

impl Drop for InitializationCleanup {
    fn drop(&mut self) {
        if self.armed {
            // dropping the stored senders wakes every waiter with a channel-closed error. this
            // also handles task cancellation and unwinding without leaving a zombie entry.
            let _ = self.entries.remove_if(&self.id, |_, entry| {
                matches!(
                    entry,
                    PoolEntry::Connecting(connecting)
                        if Arc::ptr_eq(&connecting.generation, &self.generation)
                )
            });
        }
    }
}

struct InitializedConnection {
    executor: Arc<PooledWebSocketExecutor>,
    owner: ConnectionOwner,
}

async fn initialize_connection(
    entries: &Arc<PoolEntries>,
    id: &WebSocketConnectionId,
    init: WebSocketInit,
) -> Result<InitializedConnection, PoolInitError> {
    let wsconn = websocket_client::connect(&init.endpoint, init.tls_config)
        .await
        .map_err(|error| PoolInitError::Connect(error.to_string()))?;
    let client = WsClient::new(wsconn);
    let init_payload = (!init.headers.is_empty()).then(|| init.headers.into());
    let mut client = client
        .init(init_payload)
        .await
        .map_err(|error| PoolInitError::Handshake(error.to_string()))?;
    let dispatcher_done = client.take_dispatcher_done();
    let (commands, task_commands) = mpsc::channel(init.buffer_capacity);
    let endpoint = Arc::<str>::from(init.endpoint.to_string());
    let executor = Arc::new(PooledWebSocketExecutor {
        commands,
        telemetry_context: init.telemetry_context.clone(),
        buffer_capacity: init.buffer_capacity,
        entries: Arc::downgrade(entries),
        id: id.clone(),
        endpoint_uri: init.endpoint,
        endpoint: endpoint.clone(),
    });
    let owner = ConnectionOwner {
        client,
        dispatcher_done,
        commands: task_commands,
        executor: Arc::downgrade(&executor),
        telemetry_context: init.telemetry_context,
        subgraph_name: id.subgraph_name.clone(),
        endpoint,
        idle_timeout: init.idle_timeout,
    };

    Ok(InitializedConnection { executor, owner })
}

struct ConnectionCommand {
    payload: SubscribePayload,
    custom_scalar_paths: Option<CustomScalarPaths>,
    responses: mpsc::Sender<SubscriptionItem>,
    ready: oneshot::Sender<Result<(), WsClientError>>,
}

pub struct PooledWebSocketExecutor {
    commands: mpsc::Sender<ConnectionCommand>,
    telemetry_context: Arc<TelemetryContext>,
    buffer_capacity: usize,
    entries: Weak<PoolEntries>,
    id: WebSocketConnectionId,
    endpoint_uri: Uri,
    endpoint: Arc<str>,
}

impl PooledWebSocketExecutor {
    fn evict_if_current(&self) {
        if let Some(entries) = self.entries.upgrade() {
            entries.remove_if(&self.id, |_, entry| {
                matches!(
                    entry,
                    PoolEntry::Initialized(current)
                        if current.commands.same_channel(&self.commands)
                )
            });
        }
    }

    async fn submit(
        &self,
        execution_request: SubgraphExecutionRequest<'_>,
        response_capacity: usize,
    ) -> Result<mpsc::Receiver<SubscriptionItem>, SubgraphExecutorError> {
        // reserve first so canceled callers do not build payloads or occupy response buffers while
        // waiting behind a full shared command queue.
        let permit = self.commands.reserve().await.map_err(|_| {
            self.evict_if_current();
            SubgraphExecutorError::WebSocketArbiterChannelClosed
        })?;

        let custom_scalar_paths = execution_request.custom_scalar_paths.cloned();
        let payload = execution_request.into();
        let (responses, receiver) = mpsc::channel(response_capacity);
        let (ready, ready_rx) = oneshot::channel();
        permit.send(ConnectionCommand {
            payload,
            custom_scalar_paths,
            responses,
            ready,
        });

        match ready_rx.await {
            Ok(Ok(())) => Ok(receiver),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => {
                self.evict_if_current();
                Err(SubgraphExecutorError::WebSocketArbiterChannelClosed)
            }
        }
    }
}

#[async_trait]
impl SubgraphExecutor for PooledWebSocketExecutor {
    fn executor_name(&self) -> &str {
        "pooled-websocket"
    }

    fn endpoint(&self) -> &Uri {
        &self.endpoint_uri
    }

    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        _plugin_req_state: Option<&'a PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        let _operation_guard = self
            .telemetry_context
            .metrics
            .subscriptions
            .active_subgraph_operation(&self.id.subgraph_name);
        let operation = async {
            let mut responses = self.submit(execution_request, 1).await?;
            responses.recv().await.ok_or_else(|| {
                SubgraphExecutorError::WebSocketStreamClosedEmpty(self.endpoint.to_string())
            })?
        };
        match timeout {
            Some(timeout) => tokio::time::timeout(timeout, operation).await?,
            None => operation.await,
        }
    }

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        _timeout: Option<Duration>,
    ) -> Result<BoxStream<'static, SubscriptionItem>, SubgraphExecutorError> {
        // timeout covers queueing and the subscribe write, not the lifetime of the returned stream.
        let mut responses = self.submit(execution_request, self.buffer_capacity).await?;
        let operation_guard = self
            .telemetry_context
            .metrics
            .subscriptions
            .active_subgraph_operation(&self.id.subgraph_name);
        Ok(Box::pin(async_stream::stream! {
            let _operation_guard = operation_guard;
            while let Some(item) = responses.recv().await {
                yield item;
            }
        }))
    }
}

#[async_trait]
impl SubgraphExecutor for Arc<PooledWebSocketExecutor> {
    fn executor_name(&self) -> &str {
        self.as_ref().executor_name()
    }

    fn endpoint(&self) -> &Uri {
        self.as_ref().endpoint()
    }

    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: Option<&'a PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        self.as_ref()
            .execute(execution_request, timeout, plugin_req_state)
            .await
    }

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> Result<BoxStream<'static, SubscriptionItem>, SubgraphExecutorError> {
        self.as_ref().subscribe(execution_request, timeout).await
    }
}

struct OperationCompletionGuard(mpsc::UnboundedSender<()>);

impl Drop for OperationCompletionGuard {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

enum ConnectionShutdown {
    Idle,
    Dispatcher(WsClientError),
    CommandsClosed,
}

impl ConnectionShutdown {
    fn command_error(&self) -> WsClientError {
        match self {
            Self::Idle => WsClientError::ConnectionClosed,
            Self::Dispatcher(error) => error.clone(),
            Self::CommandsClosed => WsClientError::MessageDispatcherClosed,
        }
    }
}

/// Owns the non-`Send` WebSocket client and serializes all writes to its protocol state.
///
/// The owner closes command intake before eviction and drains every command accepted before the
/// close. This gives each submitter either a started operation or an explicit error, including when
/// idle expiration races with command submission.
struct ConnectionOwner {
    client: WsClient<crate::executors::websocket_client::Initialized>,
    dispatcher_done: ntex::channel::oneshot::Receiver<WsClientError>,
    commands: mpsc::Receiver<ConnectionCommand>,
    executor: Weak<PooledWebSocketExecutor>,
    telemetry_context: Arc<TelemetryContext>,
    subgraph_name: Arc<str>,
    endpoint: Arc<str>,
    idle_timeout: Duration,
}

impl ConnectionOwner {
    fn evict(&self) {
        if let Some(executor) = self.executor.upgrade() {
            executor.evict_if_current();
        }
    }

    async fn run(mut self) {
        let telemetry_context = self.telemetry_context.clone();
        let subgraph_name = self.subgraph_name.clone();
        let _connection_guard = telemetry_context
            .metrics
            .subscriptions
            .active_subgraph_connection(&subgraph_name, SubscriptionTransport::WebSocket);
        let (completed_tx, mut completed_rx) = mpsc::unbounded_channel();
        let mut active_operations = 0usize;
        let idle_timer = tokio::time::sleep(self.idle_timeout);
        tokio::pin!(idle_timer);

        let shutdown = 'connection: loop {
            tokio::select! {
                command = self.commands.recv() => {
                    let Some(ConnectionCommand {
                        payload,
                        custom_scalar_paths,
                        responses,
                        ready,
                    }) = command else {
                        break ConnectionShutdown::CommandsClosed;
                    };

                    if active_operations == 0 {
                        idle_timer
                            .as_mut()
                            .reset(Instant::now() + self.idle_timeout);
                    }
                    if responses.is_closed() {
                        continue;
                    }

                    // writes stay serialized because WsClient mutates one subscription registry
                    // and one sink. the cancellation branch is safe because WsClient removes a
                    // partially registered operation with its own drop guard.
                    let subscribe_result = {
                        let subscribe = self.client.subscribe(payload, custom_scalar_paths);
                        tokio::pin!(subscribe);
                        tokio::select! {
                            result = &mut subscribe => Some(result),
                            _ = responses.closed() => None,
                            dispatcher = &mut self.dispatcher_done => {
                                let error = dispatcher
                                    .unwrap_or(WsClientError::MessageDispatcherClosed);
                                let _ = ready.send(Err(error.clone()));
                                break 'connection ConnectionShutdown::Dispatcher(error);
                            }
                        }
                    };

                    let Some(subscribe_result) = subscribe_result else {
                        continue;
                    };
                    match subscribe_result {
                        Ok(stream) => {
                            // if the caller timed out after the write, dropping the stream sends
                            // complete immediately instead of starting work nobody can consume.
                            if ready.send(Ok(())).is_err() {
                                drop(stream);
                                continue;
                            }

                            active_operations += 1;
                            let completion_guard =
                                OperationCompletionGuard(completed_tx.clone());
                            let telemetry_context = self.telemetry_context.clone();
                            let subgraph_name = self.subgraph_name.clone();
                            let endpoint = self.endpoint.clone();
                            rt::spawn(async move {
                                let _completion_guard = completion_guard;
                                drain_into(
                                    stream.map(|item| item.map_err(SubgraphExecutorError::from)),
                                    responses,
                                    &telemetry_context,
                                    SubscriptionTransport::WebSocket,
                                    &subgraph_name,
                                    &endpoint,
                                )
                                .await;
                            });
                        }
                        Err(error) => {
                            let _ = ready.send(Err(error));
                        }
                    }
                }
                Some(()) = completed_rx.recv(), if active_operations > 0 => {
                    active_operations -= 1;
                    if active_operations == 0 {
                        idle_timer
                            .as_mut()
                            .reset(Instant::now() + self.idle_timeout);
                    }
                }
                dispatcher = &mut self.dispatcher_done => {
                    break ConnectionShutdown::Dispatcher(
                        dispatcher.unwrap_or(WsClientError::MessageDispatcherClosed),
                    );
                }
                _ = &mut idle_timer, if active_operations == 0 => {
                    break ConnectionShutdown::Idle;
                }
            }
        };

        // close first so new reservations fail, evict next so lookups miss, then wait for any
        // reservation acquired before close and reject every command it committed.
        self.commands.close();
        self.evict();
        while let Some(command) = self.commands.recv().await {
            let _ = command.ready.send(Err(shutdown.command_error()));
        }

        if matches!(shutdown, ConnectionShutdown::Idle) {
            self.telemetry_context
                .metrics
                .subscriptions
                .record_websocket_pool_idle_expiration();
        }
    }
}

impl Drop for ConnectionOwner {
    fn drop(&mut self) {
        // this is the unwind/cancellation path; normal shutdown already performed both operations.
        self.commands.close();
        self.evict();
    }
}
