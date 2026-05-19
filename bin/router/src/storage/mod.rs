use crate::storage::error::StorageError;
use async_trait::async_trait;
use hive_router_config::storage::{StorageConfigMap, StorageSourceConfig};
use object_store::path::Path;
use std::{collections::HashMap, sync::Arc};
use tracing::debug;

pub mod error;
pub mod s3_runtime;
pub mod utils;

pub struct StorageManager {
    storage_runtimes: HashMap<String, Arc<Box<dyn StorageRuntime>>>,
}

impl StorageManager {
    pub fn new(config_map: &StorageConfigMap) -> Result<Self, StorageError> {
        let mut storage_runtimes: HashMap<String, Arc<Box<dyn StorageRuntime>>> = HashMap::new();

        for (id, config) in config_map {
            debug!(storage_id = id, config = ?config, "creating storage runtime");

            storage_runtimes.insert(
                id.to_string(),
                Arc::new(resolve_storage_config(id, config)?),
            );
        }

        Ok(Self { storage_runtimes })
    }

    pub fn get_storage_runtime(&self, id: &str) -> Option<Arc<Box<dyn StorageRuntime>>> {
        self.storage_runtimes.get(id).map(Arc::clone)
    }
}

#[async_trait]
pub trait StorageRuntime: Send + Sync + 'static {
    fn identifier(&self) -> &str;
    async fn get(&self, location: &Path) -> Result<(String, Option<String>), StorageError>;
    async fn get_if_none_changed(
        &self,
        location: &Path,
        if_none_match: Option<String>,
    ) -> Result<StorageGetResult, StorageError>;
}

pub enum StorageGetResult {
    NotModified,
    Modified {
        contents: String,
        etag: Option<String>,
    },
}

pub fn resolve_storage_config(
    id: &str,
    config: &StorageSourceConfig,
) -> Result<Box<dyn StorageRuntime>, StorageError> {
    match config {
        StorageSourceConfig::S3(s3_config) => {
            Ok(Box::new(s3_runtime::S3StorageRuntime::new(id, s3_config)?))
        }
    }
}
