use arc_swap::ArcSwap;
use async_trait::async_trait;
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{Mutex, Notify};
use tracing::{info, warn};

use hive_router_config::persisted_documents::PersistedDocumentsFileStorageConfig;
use hive_router_internal::background_tasks::{BackgroundTask, CancellationToken};

use super::{
    PersistedDocumentResolveInput, PersistedDocumentResolver, PersistedDocumentResolverError,
    ResolvedDocument,
};

const RELOAD_EVENT_DEBOUNCE: Duration = Duration::from_millis(150);

// In-memory map used by the file manifest resolver.
// Values are Arc-backed so lookups only clone cheap references.
struct DocumentsById(HashMap<String, Arc<str>>);

impl Deref for DocumentsById {
    type Target = HashMap<String, Arc<str>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[async_trait]
impl PersistedDocumentResolver for FileManifestResolver {
    async fn resolve(
        &self,
        input: PersistedDocumentResolveInput<'_>,
    ) -> Result<ResolvedDocument, PersistedDocumentResolverError> {
        // File manifests are keyed only by persisted document id.
        // Client identity is ignored for this source type.
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

pub struct FileManifestResolver {
    manifest_path: String,
    // Snapshot of currently active documents for lock-free reads.
    documents: ArcSwap<DocumentsById>,
    // Signals a potential file change
    dirty: Arc<AtomicBool>,
    // Ensures at-most-one reload in flight so watcher events do not race
    // and publish snapshots out of order.
    reload_guard: Mutex<()>,
    // Notification channel from watcher callback to background reload task.
    reload_signal: Arc<Notify>,
    watcher: Option<RecommendedWatcher>,
}

// Background task wrapper registered in the shared task manager.
pub struct FileManifestReloadTask(pub Arc<FileManifestResolver>);

impl Deref for FileManifestReloadTask {
    type Target = Arc<FileManifestResolver>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Deserialize)]
struct ApolloPersistedQueryManifest<'a> {
    #[serde(borrow)]
    format: Cow<'a, str>,
    version: u8,
    #[serde(borrow)]
    operations: Vec<ApolloPersistedQueryOperation<'a>>,
}

#[derive(Deserialize)]
struct ApolloPersistedQueryOperation<'a> {
    #[serde(borrow)]
    id: Cow<'a, str>,
    #[serde(borrow)]
    body: Cow<'a, str>,
}

type KeyValueManifest<'a> = HashMap<Cow<'a, str>, Cow<'a, str>>;

#[derive(Deserialize)]
#[serde(untagged)]
#[serde(bound(deserialize = "'de: 'a"))]
enum PersistedDocumentsManifest<'a> {
    Apollo(ApolloPersistedQueryManifest<'a>),
    KeyValue(KeyValueManifest<'a>),
}

#[derive(Debug, Error)]
pub enum FileResolverError {
    #[error("failed to read persisted documents manifest at '{path}': {message}")]
    ReadManifest { path: String, message: String },
    #[error("failed to parse persisted documents manifest at '{path}': {message}")]
    ParseManifest { path: String, message: String },
    #[error("unsupported apollo manifest format. Expected 'apollo-persisted-query-manifest', received '{format}'")]
    UnsupportedApolloManifestFormat { format: String },
    #[error("unsupported apollo manifest version. Expected '1', received '{version}'")]
    UnsupportedApolloManifestVersion { version: u8 },
    #[error("failed to initialize persisted documents file watcher for '{path}': {message}")]
    WatcherInit { path: String, message: String },
    #[error("failed to watch persisted documents path '{path}': {message}")]
    WatcherWatchPath { path: String, message: String },
}

impl FileManifestResolver {
    pub async fn from_storage_config(
        config: &PersistedDocumentsFileStorageConfig,
    ) -> Result<Self, PersistedDocumentResolverError> {
        let manifest_path = config.path.absolute.clone();
        let documents = Self::read_manifest_documents(&manifest_path).await?;
        let dirty = Arc::new(AtomicBool::new(false));
        let reload_signal = Arc::new(Notify::new());
        let watcher = if config.watch {
            Some(Self::create_watcher(
                &manifest_path,
                Arc::clone(&dirty),
                Arc::clone(&reload_signal),
            )?)
        } else {
            None
        };

        Ok(Self {
            manifest_path,
            documents: ArcSwap::from_pointee(documents),
            dirty,
            reload_guard: Mutex::new(()),
            reload_signal,
            watcher,
        })
    }

    pub(crate) fn has_watcher(&self) -> bool {
        self.watcher.is_some()
    }

    fn create_watcher(
        manifest_path: &str,
        dirty: Arc<AtomicBool>,
        reload_signal: Arc<Notify>,
    ) -> Result<RecommendedWatcher, PersistedDocumentResolverError> {
        let path = Path::new(manifest_path);
        let manifest_path_buf = PathBuf::from(manifest_path);
        // Watch the parent directory so replace/rename save patterns are observed.
        let watch_target = path.parent().unwrap_or(path);

        let mut watcher = match RecommendedWatcher::new(
            move |result: notify::Result<notify::Event>| {
                let should_signal_reload = match result {
                    Ok(event) => {
                        let is_relevant_kind = matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        );
                        let touches_manifest =
                            event.paths.iter().any(|path| path == &manifest_path_buf);
                        is_relevant_kind && touches_manifest
                    }
                    Err(err) => {
                        warn!(error = %err, "persisted documents watcher event failed");
                        true
                    }
                };

                if should_signal_reload {
                    dirty.store(true, Ordering::Relaxed);
                    reload_signal.notify_one();
                }
            },
            NotifyConfig::default(),
        ) {
            Ok(watcher) => watcher,
            Err(err) => {
                return Err(FileResolverError::WatcherInit {
                    path: manifest_path.to_string(),
                    message: err.to_string(),
                }
                .into());
            }
        };

        if let Err(err) = watcher.watch(watch_target, RecursiveMode::NonRecursive) {
            return Err(FileResolverError::WatcherWatchPath {
                path: manifest_path.to_string(),
                message: err.to_string(),
            }
            .into());
        }

        Ok(watcher)
    }

    // Keeps last known good snapshot active when reload fails
    pub(crate) async fn reload_if_needed(&self) -> Result<(), PersistedDocumentResolverError> {
        let _reload_guard = self.reload_guard.lock().await;

        if !self.dirty.swap(false, Ordering::Relaxed) {
            return Ok(());
        }

        let documents = Self::read_manifest_documents(&self.manifest_path).await?;
        self.documents.store(Arc::new(documents));
        info!(
            manifest_path = self.manifest_path,
            "reloaded persisted documents manifest",
        );
        Ok(())
    }

    async fn read_manifest_documents(
        manifest_path: &str,
    ) -> Result<DocumentsById, PersistedDocumentResolverError> {
        tokio::fs::read(manifest_path)
            .await
            .map_err(|err| {
                PersistedDocumentResolverError::from(FileResolverError::ReadManifest {
                    path: manifest_path.to_string(),
                    message: err.to_string(),
                })
            })
            .and_then(|raw| {
                let manifest: PersistedDocumentsManifest<'_> =
                    sonic_rs::from_slice(&raw).map_err(|err| FileResolverError::ParseManifest {
                        path: manifest_path.to_string(),
                        message: err.to_string(),
                    })?;

                manifest.try_into()
            })
    }
}

#[async_trait]
impl BackgroundTask for FileManifestReloadTask {
    fn id(&self) -> &str {
        "persisted-documents-file-reloader"
    }

    async fn run(&self, token: CancellationToken) {
        // Watcher events are debounced to reduce noisy save/update actions
        while token
            .run_until_cancelled(async {
                self.reload_signal.notified().await;
                tokio::time::sleep(RELOAD_EVENT_DEBOUNCE).await;
            })
            .await
            .is_some()
        {
            if let Err(err) = self.reload_if_needed().await {
                warn!(error = %err, "persisted documents background reload failed");
            }
        }
    }
}

impl<'a> TryFrom<PersistedDocumentsManifest<'a>> for DocumentsById {
    type Error = PersistedDocumentResolverError;

    fn try_from(value: PersistedDocumentsManifest<'a>) -> Result<Self, Self::Error> {
        match value {
            PersistedDocumentsManifest::Apollo(manifest) => manifest.try_into(),
            PersistedDocumentsManifest::KeyValue(manifest) => Ok(manifest.into()),
        }
    }
}

impl<'a> TryFrom<ApolloPersistedQueryManifest<'a>> for DocumentsById {
    type Error = PersistedDocumentResolverError;

    fn try_from(manifest: ApolloPersistedQueryManifest<'a>) -> Result<Self, Self::Error> {
        if manifest.format != "apollo-persisted-query-manifest" {
            return Err(FileResolverError::UnsupportedApolloManifestFormat {
                format: manifest.format.into_owned(),
            }
            .into());
        }

        if manifest.version != 1 {
            return Err(FileResolverError::UnsupportedApolloManifestVersion {
                version: manifest.version,
            }
            .into());
        }

        Ok(DocumentsById(
            manifest
                .operations
                .into_iter()
                .map(|op| (op.id.into_owned(), Arc::<str>::from(op.body)))
                .collect::<HashMap<_, _>>(),
        ))
    }
}

impl<'a> From<KeyValueManifest<'a>> for DocumentsById {
    fn from(manifest: KeyValueManifest<'a>) -> Self {
        DocumentsById(
            manifest
                .into_iter()
                .map(|(id, text)| (id.into_owned(), Arc::<str>::from(text)))
                .collect(),
        )
    }
}
