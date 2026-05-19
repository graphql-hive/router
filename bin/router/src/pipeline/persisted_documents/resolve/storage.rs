use arc_swap::ArcSwap;
use async_trait::async_trait;
use hive_router_config::persisted_documents::PersistedDocumentsStorageRefConfig;
use hive_router_internal::background_tasks::BackgroundTask;
use object_store::path::Path;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::{
    pipeline::persisted_documents::resolve::{
        shared_file_manifest::{parse_manifest, DocumentsById},
        PersistedDocumentResolveInput, PersistedDocumentResolver, PersistedDocumentResolverError,
        ResolvedDocument,
    },
    storage::{StorageGetResult, StorageRuntime},
};

pub struct StorageResolver {
    documents: ArcSwap<DocumentsById>,
    location: Path,
    storage: Arc<Box<dyn StorageRuntime>>,
    last_etag: RwLock<Option<String>>,
}

impl StorageResolver {
    pub async fn from_storage_config(
        config: &PersistedDocumentsStorageRefConfig,
        storage: Arc<Box<dyn StorageRuntime>>,
    ) -> Result<Self, PersistedDocumentResolverError> {
        let location = Path::from(config.location.as_str());
        let (raw_manifest, etag) = storage.get(&location).await?;
        let parsed_manifest = parse_manifest(location.as_ref(), raw_manifest.as_bytes())
            .map_err(PersistedDocumentResolverError::FileManifest)?;
        let documents: DocumentsById = parsed_manifest.try_into()?;

        Ok(Self {
            location,
            storage,
            last_etag: RwLock::new(etag),
            documents: ArcSwap::from_pointee(documents),
        })
    }

    pub async fn reload_if_needed(&self) -> Result<(), PersistedDocumentResolverError> {
        let latest_etag = {
            let guard = self.last_etag.read().await;
            guard.clone()
        };

        let result = self
            .storage
            .get_if_none_changed(&self.location, latest_etag)
            .await?;

        match result {
            StorageGetResult::NotModified => {
                debug!("persisted documents store was not modified");
            }
            StorageGetResult::Modified { contents, etag } => {
                let parsed_manifest = parse_manifest(self.location.as_ref(), contents.as_bytes())
                    .map_err(PersistedDocumentResolverError::FileManifest)?;
                let documents: DocumentsById = parsed_manifest.try_into()?;
                self.documents.store(Arc::new(documents));
                *self.last_etag.write().await = etag;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl PersistedDocumentResolver for StorageResolver {
    async fn resolve(
        &self,
        input: PersistedDocumentResolveInput<'_>,
    ) -> Result<ResolvedDocument, PersistedDocumentResolverError> {
        let text = self
            .documents
            .load()
            .get(input.persisted_document_id.as_ref())
            .cloned()
            .ok_or_else(|| {
                PersistedDocumentResolverError::NotFound(input.persisted_document_id.to_string())
            })?;

        Ok(ResolvedDocument { text })
    }
}

pub struct StorageManifestReloadTask {
    loader: Arc<StorageResolver>,
    poll_interval: Duration,
}

impl StorageManifestReloadTask {
    pub fn new(loader: Arc<StorageResolver>, poll_interval: Duration) -> Self {
        Self {
            loader,
            poll_interval,
        }
    }
}

#[async_trait]
impl BackgroundTask for StorageManifestReloadTask {
    fn id(&self) -> &str {
        "persisted-documents-storage-reloader"
    }

    async fn run(&self, token: CancellationToken) {
        loop {
            if token.is_cancelled() {
                break;
            }

            let error = self.loader.reload_if_needed().await;

            if let Err(err) = error {
                tracing::error!(error = %err, "failed to reload persisted documents manifest from storage");
            }

            ntex::time::sleep(self.poll_interval).await;
        }
    }
}
