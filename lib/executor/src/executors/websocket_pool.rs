use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use dashmap::DashMap;
use futures::{channel::oneshot, stream::BoxStream, StreamExt};
use hive_router_internal::{
    telemetry::metrics::subscription_metrics::SubscriptionTransport, telemetry::TelemetryContext,
};
use hive_router_query_planner::planner::plan_nodes::CustomScalarPaths;
use http::Uri;
use ntex::rt;
use tokio::sync::mpsc;

use crate::{
    executors::{
        common::{ConnectionFingerprint, SubgraphExecutionRequest, SubgraphExecutor},
        error::SubgraphExecutorError,
        graphql_transport_ws::SubscribePayload,
        subscription_buffer::{drain_into, try_send_or_drop},
        websocket_client::{self, WsClient, WsClientError},
    },
    plugin_context::PluginRequestState,
    response::subgraph_response::SubgraphResponse,
};

type SubscriptionItem = Result<SubgraphResponse<'static>, SubgraphExecutorError>;
type InitResult = Result<Arc<PooledWebSocketExecutor>, PoolInitError>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WebSocketConnectionId {
    pub endpoint: Uri,
    pub fingerprint: ConnectionFingerprint,
}

// TODO: use thiserror and transparent and whatever to inherit errors #[from]
#[derive(Clone)]
enum PoolInitError {
    Connect(String),
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
    waiters: Vec<oneshot::Sender<InitResult>>,
}

enum PoolEntry {
    Connecting(ConnectingEntry),
    Initialized(Arc<PooledWebSocketExecutor>),
}

pub struct WebSocketInit {
    pub headers: http::HeaderMap,
    pub tls_config: Option<Arc<rustls::ClientConfig>>,
    pub subgraph_name: String,
    pub buffer_capacity: usize,
    pub idle_timeout: Duration,
    pub telemetry_context: Arc<TelemetryContext>,
}

pub struct WebSocketPool {
    entries: Arc<DashMap<WebSocketConnectionId, PoolEntry>>,
}

impl Default for WebSocketPool {
    fn default() -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
        }
    }
}

impl WebSocketPool {
    pub fn get_initialized(
        &self,
        id: &WebSocketConnectionId,
    ) -> Option<Arc<PooledWebSocketExecutor>> {
        self.entries.get(id).and_then(|entry| match entry.value() {
            PoolEntry::Initialized(executor) => Some(executor.clone()),
            PoolEntry::Connecting(_) => None,
        })
    }

    pub async fn get_or_initialize(
        &self,
        id: WebSocketConnectionId,
        init: WebSocketInit,
    ) -> Result<Arc<PooledWebSocketExecutor>, SubgraphExecutorError> {
        let (wait_tx, wait_rx) = oneshot::channel();
        let initialize = match self.entries.entry(id.clone()) {
            dashmap::mapref::entry::Entry::Occupied(mut entry) => match entry.get_mut() {
                PoolEntry::Initialized(executor) => return Ok(executor.clone()),
                PoolEntry::Connecting(connecting) => {
                    init.telemetry_context
                        .metrics
                        .subscriptions
                        .record_websocket_pool_initialization_joined();
                    connecting.waiters.push(wait_tx);
                    false
                }
            },
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                init.telemetry_context
                    .metrics
                    .subscriptions
                    .record_websocket_pool_initialization_started();
                entry.insert(PoolEntry::Connecting(ConnectingEntry {
                    waiters: vec![wait_tx],
                }));
                true
            }
        };

        if initialize {
            let entries = self.entries.clone();
            let task_id = id.clone();
            let telemetry_context = init.telemetry_context.clone();
            rt::spawn(async move {
                let initialized =
                    initialize_connection(entries.clone(), task_id.clone(), init).await;
                if initialized.is_err() {
                    telemetry_context
                        .metrics
                        .subscriptions
                        .record_websocket_pool_initialization_failed();
                }
                let waiters = match entries.remove(&task_id) {
                    Some((_, PoolEntry::Connecting(connecting))) => connecting.waiters,
                    Some((_, initialized @ PoolEntry::Initialized(_))) => {
                        entries.insert(task_id.clone(), initialized);
                        Vec::new()
                    }
                    None => Vec::new(),
                };

                let result = match initialized {
                    Ok((executor, start_owner)) => {
                        entries.insert(task_id, PoolEntry::Initialized(executor.clone()));
                        let _ = start_owner.send(());
                        Ok(executor)
                    }
                    Err(error) => Err(error),
                };
                for waiter in waiters {
                    let _ = waiter.send(result.clone());
                }
            });
        }

        wait_rx
            .await
            .map_err(|_| SubgraphExecutorError::WebSocketArbiterChannelClosed)?
            .map_err(|error| error.into_executor_error(&id.endpoint))
    }
}

async fn initialize_connection(
    entries: Arc<DashMap<WebSocketConnectionId, PoolEntry>>,
    id: WebSocketConnectionId,
    init: WebSocketInit,
) -> Result<(Arc<PooledWebSocketExecutor>, oneshot::Sender<()>), PoolInitError> {
    let wsconn = websocket_client::connect(&id.endpoint, init.tls_config)
        .await
        .map_err(|error| PoolInitError::Connect(error.to_string()))?;
    let client = WsClient::new(wsconn);
    let headers = init.headers;
    let mut client = client
        .init((!headers.is_empty()).then(|| headers.into()))
        .await
        .map_err(|error| PoolInitError::Handshake(error.to_string()))?;
    let dispatcher_done = client.take_dispatcher_done();
    let (commands, task_commands) = mpsc::channel(init.buffer_capacity);
    let executor = Arc::new(PooledWebSocketExecutor {
        endpoint: id.endpoint.clone(),
        commands,
        telemetry_context: init.telemetry_context.clone(),
        subgraph_name: init.subgraph_name.clone(),
        buffer_capacity: init.buffer_capacity,
        entries: Arc::downgrade(&entries),
        id: id.clone(),
    });

    let task_executor = executor.clone();
    let (start_owner, start_owner_rx) = oneshot::channel();
    rt::spawn(async move {
        if start_owner_rx.await.is_err() {
            return;
        }
        let _connection_guard = init
            .telemetry_context
            .metrics
            .subscriptions
            .active_subgraph_connection(&init.subgraph_name, SubscriptionTransport::WebSocket);
        own_connection(
            client,
            dispatcher_done,
            task_commands,
            task_executor,
            id,
            entries,
            init.idle_timeout,
        )
        .await;
    });

    Ok((executor, start_owner))
}

enum ConnectionCommand {
    Subscribe {
        payload: SubscribePayload,
        custom_scalar_paths: Option<CustomScalarPaths>,
        responses: mpsc::Sender<SubscriptionItem>,
    },
}

pub struct PooledWebSocketExecutor {
    endpoint: Uri,
    commands: mpsc::Sender<ConnectionCommand>,
    telemetry_context: Arc<TelemetryContext>,
    subgraph_name: String,
    buffer_capacity: usize,
    entries: std::sync::Weak<DashMap<WebSocketConnectionId, PoolEntry>>,
    id: WebSocketConnectionId,
}

impl PooledWebSocketExecutor {
    async fn submit(
        &self,
        execution_request: SubgraphExecutionRequest<'_>,
        response_capacity: usize,
    ) -> Result<mpsc::Receiver<SubscriptionItem>, SubgraphExecutorError> {
        let custom_scalar_paths = execution_request.custom_scalar_paths.cloned();
        let payload = execution_request.into();
        let (responses, receiver) = mpsc::channel(response_capacity);
        if self
            .commands
            .send(ConnectionCommand::Subscribe {
                payload,
                custom_scalar_paths,
                responses,
            })
            .await
            .is_err()
        {
            if let Some(entries) = self.entries.upgrade() {
                entries.remove_if(&self.id, |_, entry| {
                    matches!(
                        entry,
                        PoolEntry::Initialized(current)
                            if current.commands.same_channel(&self.commands)
                    )
                });
            }
            return Err(SubgraphExecutorError::WebSocketArbiterChannelClosed);
        }
        Ok(receiver)
    }
}

#[async_trait]
impl SubgraphExecutor for PooledWebSocketExecutor {
    fn executor_name(&self) -> &str {
        "pooled-websocket"
    }

    fn endpoint(&self) -> &Uri {
        &self.endpoint
    }

    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        _plugin_req_state: Option<&'a PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        let endpoint = self.endpoint.to_string();
        let _operation_guard = self
            .telemetry_context
            .metrics
            .subscriptions
            .active_subgraph_operation(&self.subgraph_name);
        let operation = async {
            let mut responses = self.submit(execution_request, 1).await?;
            responses
                .recv()
                .await
                .ok_or(SubgraphExecutorError::WebSocketStreamClosedEmpty(endpoint))?
        };
        match timeout {
            Some(timeout) => tokio::time::timeout(timeout, operation).await?,
            None => operation.await,
        }
    }

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> Result<BoxStream<'static, SubscriptionItem>, SubgraphExecutorError> {
        let submit = self.submit(execution_request, self.buffer_capacity);
        let mut responses = match timeout {
            Some(timeout) => tokio::time::timeout(timeout, submit).await??,
            None => submit.await?,
        };
        let operation_guard = self
            .telemetry_context
            .metrics
            .subscriptions
            .active_subgraph_operation(&self.subgraph_name);
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
        (**self).executor_name()
    }

    fn endpoint(&self) -> &Uri {
        (**self).endpoint()
    }

    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: Option<&'a PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError> {
        (**self)
            .execute(execution_request, timeout, plugin_req_state)
            .await
    }

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> Result<BoxStream<'static, SubscriptionItem>, SubgraphExecutorError> {
        (**self).subscribe(execution_request, timeout).await
    }
}

async fn own_connection(
    mut client: WsClient<crate::executors::websocket_client::Initialized>,
    mut dispatcher_done: ntex::channel::oneshot::Receiver<WsClientError>,
    mut commands: mpsc::Receiver<ConnectionCommand>,
    executor: Arc<PooledWebSocketExecutor>,
    id: WebSocketConnectionId,
    entries: Arc<DashMap<WebSocketConnectionId, PoolEntry>>,
    idle_timeout: Duration,
) {
    let (completed_tx, mut completed_rx) = mpsc::unbounded_channel();
    let mut active = 0usize;
    let mut idle_expired = false;
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(ConnectionCommand::Subscribe { payload, custom_scalar_paths, responses }) => {
                    match client.subscribe(payload, custom_scalar_paths).await {
                        Ok(stream) => {
                            active += 1;
                            let telemetry_context = executor.telemetry_context.clone();
                            let subgraph_name = executor.subgraph_name.clone();
                            let endpoint = executor.endpoint.to_string();
                            let completed_tx = completed_tx.clone();
                            rt::spawn(async move {
                                drain_into(
                                    stream.map(|item| item.map_err(SubgraphExecutorError::from)),
                                    responses,
                                    &telemetry_context,
                                    SubscriptionTransport::WebSocket,
                                    &subgraph_name,
                                    &endpoint,
                                ).await;
                                let _ = completed_tx.send(());
                            });
                        }
                        Err(error) => {
                            try_send_or_drop(
                                &responses,
                                Err(error.into()),
                                &executor.telemetry_context,
                                SubscriptionTransport::WebSocket,
                                &executor.subgraph_name,
                                &executor.endpoint.to_string(),
                            );
                        }
                    }
                }
                None => break,
            },
            Some(()) = completed_rx.recv(), if active > 0 => active -= 1,
            dispatcher = &mut dispatcher_done => {
                let error = dispatcher.unwrap_or(WsClientError::MessageDispatcherClosed);
                while let Ok(ConnectionCommand::Subscribe { responses, .. }) = commands.try_recv() {
                    let _ = responses.try_send(Err(error.clone().into()));
                }
                break;
            }
            _ = tokio::time::sleep(idle_timeout), if active == 0 => {
                idle_expired = true;
                break;
            },
        }
    }

    if idle_expired {
        executor
            .telemetry_context
            .metrics
            .subscriptions
            .record_websocket_pool_idle_expiration();
    }
    entries.remove_if(&id, |_, entry| {
        matches!(
            entry,
            PoolEntry::Initialized(current)
                if current.commands.same_channel(&executor.commands)
        )
    });
}
