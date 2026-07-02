use std::sync::Arc;

use hive_console_sdk::expressions::{CompileExpression, ProgramHints};
use hive_router_config::persisted_documents::{
    PersistedDocumentsConfig, PersistedDocumentsStorageConfig,
};
use hive_router_config::primitives::value_or_expression::ValueOrExpression;
use hive_router_internal::background_tasks::BackgroundTasksManager;
use hive_router_internal::expressions::{ToVrlValue, ValueOrProgram};
use hive_router_plan_executor::execution::client_request_details::ntex_header_map_to_vrl_value;
use ntex::web::HttpRequest;

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

pub struct PersistedDocumentsRuntime {
    pub document_id_resolver: Arc<DocumentIdResolver>,
    pub persisted_document_resolver: Option<Arc<dyn PersistedDocumentResolver>>,
    pub(crate) require_id: ValueOrProgram<bool>,
}

impl PersistedDocumentsRuntime {
    pub async fn init(
        config: &PersistedDocumentsConfig,
        graphql_endpoint: &str,
        background_tasks_mgr: &mut BackgroundTasksManager,
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
                        "
                      Failed to compile persisted document require_id expression: {err}"
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
                        background_tasks_mgr
                            .register_task(FileManifestReloadTask(resolver.clone()));
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
                                background_tasks_mgr.register_task(StorageManifestReloadTask::new(
                                    resolver.clone(),
                                    *poll_interval,
                                ));
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
