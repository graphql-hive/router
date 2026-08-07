use std::{future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use futures::{stream::FuturesUnordered, StreamExt};

use hive_console_sdk::expressions::{CompileExpression, ProgramHints};
use hive_router_config::persisted_documents::{
    PersistedDocumentsConfig, PersistedDocumentsStorageConfig,
};
use hive_router_config::primitives::value_or_expression::ValueOrExpression;
use hive_router_internal::background_tasks::{BackgroundTask, CancellationToken};
use hive_router_internal::expressions::{ToVrlValue, ValueOrProgram};
use hive_router_plan_executor::execution::client_request_details::ntex_header_map_to_vrl_value;
use ntex::web::HttpRequest;
use tokio::sync::{mpsc, Mutex};

use crate::pipeline::error::PipelineError;
use crate::pipeline::persisted_documents::extract::DocumentIdResolver;
use crate::pipeline::persisted_documents::resolve::storage::{
    StorageManifestReloadTask, StorageResolver,
};
use crate::pipeline::persisted_documents::resolve::{
    FileManifestReloadTask, FileManifestResolver, HiveCDNResolver, PersistedDocumentResolver,
    PersistedDocumentResolverError,
};
use crate::storage::StorageManager;

pub mod extract;
pub mod resolve;
pub mod types;

enum PersistedDocumentsWorkerRegistration {
    File(FileManifestReloadTask, CancellationToken),
    Storage(StorageManifestReloadTask, CancellationToken),
}

fn persisted_documents_worker(
    registration: PersistedDocumentsWorkerRegistration,
    router_shutdown: CancellationToken,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        match registration {
            PersistedDocumentsWorkerRegistration::File(task, supergraph_lifetime) => {
                let token = CancellationToken::new();
                let run = task.run(token.clone());
                tokio::pin!(run);
                tokio::select! {
                    _ = router_shutdown.cancelled() => token.cancel(),
                    _ = supergraph_lifetime.cancelled() => token.cancel(),
                    _ = &mut run => return,
                }
                run.await;
            }
            PersistedDocumentsWorkerRegistration::Storage(task, supergraph_lifetime) => {
                let token = CancellationToken::new();
                let run = task.run(token.clone());
                tokio::pin!(run);
                tokio::select! {
                    _ = router_shutdown.cancelled() => token.cancel(),
                    _ = supergraph_lifetime.cancelled() => token.cancel(),
                    _ = &mut run => return,
                }
                run.await;
            }
        }
    })
}

/// Adds manifest reload workers that are scoped to one selected supergraph runtime.
///
/// The supplied lifetime is the cancellation token owned by that runtime. A worker remains active
/// while the runtime is retained by its configured owner, runtime cache, request, WebSocket, or
/// subscription. Cancelling the lifetime removes only that runtime's workers; router shutdown is
/// handled independently by [`PersistedDocumentsBackgroundTasks`].
#[derive(Clone)]
pub struct PersistedDocumentsBackgroundTaskController {
    sender: mpsc::UnboundedSender<PersistedDocumentsWorkerRegistration>,
}

impl PersistedDocumentsBackgroundTaskController {
    fn add_file_worker(
        &self,
        task: FileManifestReloadTask,
        supergraph_lifetime: CancellationToken,
    ) {
        self.sender
            .send(PersistedDocumentsWorkerRegistration::File(
                task,
                supergraph_lifetime,
            ))
            .ok();
    }

    fn add_storage_worker(
        &self,
        task: StorageManifestReloadTask,
        supergraph_lifetime: CancellationToken,
    ) {
        self.sender
            .send(PersistedDocumentsWorkerRegistration::Storage(
                task,
                supergraph_lifetime,
            ))
            .ok();
    }
}

/// Runs all persisted-document manifest reload workers registered after router startup.
///
/// Workers are removed when their selected supergraph runtime lifetime is cancelled. This is tied
/// to the final runtime drop rather than cache eviction or owner retirement because active requests
/// and subscriptions may still retain and use that runtime.
pub struct PersistedDocumentsBackgroundTasks {
    registrations: Mutex<mpsc::UnboundedReceiver<PersistedDocumentsWorkerRegistration>>,
}

impl PersistedDocumentsBackgroundTasks {
    pub fn new() -> (PersistedDocumentsBackgroundTaskController, Self) {
        let (sender, registrations) = mpsc::unbounded_channel();
        (
            PersistedDocumentsBackgroundTaskController { sender },
            Self {
                registrations: Mutex::new(registrations),
            },
        )
    }
}

#[async_trait]
impl BackgroundTask for PersistedDocumentsBackgroundTasks {
    fn id(&self) -> &str {
        "persisted-documents-background-tasks"
    }

    async fn run(&self, router_shutdown: CancellationToken) {
        let mut registrations = self.registrations.lock().await;
        let mut workers: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>> =
            FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = router_shutdown.cancelled() => {
                    registrations.close();
                    while let Some(registration) = registrations.recv().await {
                        workers.push(persisted_documents_worker(
                            registration,
                            router_shutdown.clone(),
                        ));
                    }
                    while workers.next().await.is_some() {}
                    return;
                }
                registration = registrations.recv() => {
                    let Some(registration) = registration else {
                        while workers.next().await.is_some() {}
                        return;
                    };
                    workers.push(persisted_documents_worker(
                        registration,
                        router_shutdown.clone(),
                    ));
                }
                Some(()) = workers.next(), if !workers.is_empty() => {}
            }
        }
    }
}

pub struct PersistedDocumentsRuntime {
    pub document_id_resolver: Arc<DocumentIdResolver>,
    pub persisted_document_resolver: Option<Arc<dyn PersistedDocumentResolver>>,
    pub(crate) require_id: ValueOrProgram<bool>,
}

impl PersistedDocumentsRuntime {
    pub async fn init(
        config: &PersistedDocumentsConfig,
        graphql_endpoint: &str,
        background_tasks: &PersistedDocumentsBackgroundTaskController,
        supergraph_lifetime: CancellationToken,
        storage_manager: &Arc<StorageManager>,
    ) -> Result<Self, PersistedDocumentResolverError> {
        let document_id_resolver = Arc::new(
            DocumentIdResolver::from_config(config, graphql_endpoint).map_err(|error| {
                PersistedDocumentResolverError::Configuration(format!(
                    "failed to build persisted document extraction plan: {error}"
                ))
            })?,
        );

        let require_id = match &config.require_id {
            ValueOrExpression::Value(value) => ValueOrProgram::Value(*value),
            ValueOrExpression::Expression { expression } => {
                let program = expression.compile_expression(None).map_err(|err| {
                    PersistedDocumentResolverError::Configuration(format!(
                        "Failed to compile persisted document require_id expression: {err}"
                    ))
                })?;
                let hints = ProgramHints::from_program(&program);
                ValueOrProgram::Program(Box::new(program), hints)
            }
        };

        let persisted_document_resolver = if config.enabled {
            let storage = config
                .storage
                .as_ref()
                .ok_or(PersistedDocumentResolverError::StorageNotConfigured)?;
            match storage {
                PersistedDocumentsStorageConfig::File { config } => {
                    let resolver =
                        Arc::new(FileManifestResolver::from_storage_config(config).await?);
                    if resolver.has_watcher() {
                        background_tasks.add_file_worker(
                            FileManifestReloadTask(resolver.clone()),
                            supergraph_lifetime.clone(),
                        );
                    }
                    Some(resolver as Arc<dyn PersistedDocumentResolver>)
                }
                PersistedDocumentsStorageConfig::Hive { config } => {
                    let resolver = Arc::new(HiveCDNResolver::from_storage_config(config)?);
                    Some(resolver as Arc<dyn PersistedDocumentResolver>)
                }
                PersistedDocumentsStorageConfig::Storage { config } => {
                    match storage_manager.get_storage_runtime(&config.storage_id) {
                        Some(storage) => {
                            let resolver = Arc::new(
                                StorageResolver::from_storage_config(config, storage).await?,
                            );

                            if let Some(poll_interval) = &config.poll_interval {
                                background_tasks.add_storage_worker(
                                    StorageManifestReloadTask::new(
                                        resolver.clone(),
                                        *poll_interval,
                                    ),
                                    supergraph_lifetime.clone(),
                                );
                            }

                            Some(resolver as Arc<dyn PersistedDocumentResolver>)
                        }
                        None => {
                            return Err(PersistedDocumentResolverError::StorageNotFound(
                                config.storage_id.to_string(),
                            ));
                        }
                    }
                }
            }
        } else {
            None
        };

        Ok(Self {
            document_id_resolver,
            persisted_document_resolver,
            require_id,
        })
    }

    pub fn supports_graphql_endpoint(&self, graphql_endpoint: &str) -> bool {
        if !self.document_id_resolver.is_enabled() {
            return true;
        }

        if !self.document_id_resolver.depends_on_graphql_path() {
            return true;
        }

        let is_root_endpoint = graphql_endpoint.trim_end_matches('/').is_empty();

        // `/` can't be used as it would conflict with the path param extractor.
        // The `/:id` would match `/health` endpoint for example.
        !is_root_endpoint
    }

    pub fn require_id(&self, request: &HttpRequest) -> Result<bool, PipelineError> {
        self.require_id
            .resolve_with_hints(|hints| {
                hints.context_builder(|root| {
                    root.insert_object("request", |req| {
                        req.insert_lazy("method", || request.method().as_str().into())
                            .insert_lazy("headers", || {
                                ntex_header_map_to_vrl_value(request.headers())
                            })
                            .insert_lazy("url", || request.uri().to_vrl_value());
                    });
                })
            })
            .map_err(PipelineError::PersistedDocumentIdExpressionEvaluationError)
    }
}
