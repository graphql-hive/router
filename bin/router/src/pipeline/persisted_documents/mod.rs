use std::sync::Arc;

use hive_router_config::persisted_documents::{
    PersistedDocumentsConfig, PersistedDocumentsStorageConfig,
};
use hive_router_internal::background_tasks::BackgroundTasksManager;

use crate::pipeline::persisted_documents::extract::DocumentIdResolver;
use crate::pipeline::persisted_documents::resolve::{
    FileManifestReloadTask, FileManifestResolver, HiveCDNResolver, PersistedDocumentResolver,
    PersistedDocumentResolverError,
};

pub mod extract;
pub mod resolve;
pub mod types;

pub struct PersistedDocumentsRuntime {
    pub document_id_resolver: Arc<DocumentIdResolver>,
    pub persisted_document_resolver: Option<Arc<dyn PersistedDocumentResolver>>,
}

impl PersistedDocumentsRuntime {
    pub fn init(
        config: &PersistedDocumentsConfig,
        graphql_endpoint: &str,
        background_tasks_mgr: &mut BackgroundTasksManager,
    ) -> Result<Self, PersistedDocumentResolverError> {
        let document_id_resolver = Arc::new(
            DocumentIdResolver::from_config(config, graphql_endpoint).map_err(|error| {
                PersistedDocumentResolverError::Configuration(format!(
                    "failed to build persisted document extraction plan: {error}"
                ))
            })?,
        );

        let persisted_document_resolver = if config.enabled {
            let storage = config
                .storage
                .as_ref()
                .ok_or(PersistedDocumentResolverError::StorageNotConfigured)?;
            match storage {
                PersistedDocumentsStorageConfig::File { config } => {
                    let resolver = Arc::new(FileManifestResolver::from_storage_config(config)?);
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
            }
        } else {
            None
        };

        Ok(Self {
            document_id_resolver,
            persisted_document_resolver,
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
}
