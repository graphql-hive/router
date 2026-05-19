use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use object_store::path::Path;
use tokio::sync::RwLock;
use tracing::error;

use crate::{
    storage::{StorageGetResult, StorageRuntime},
    supergraph::base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader},
};

pub struct SupergraphStorageLoader {
    fetcher: SupergraphStorageFetcher,
    poll_interval: Option<Duration>,
}

struct SupergraphStorageFetcher {
    storage: Arc<Box<dyn StorageRuntime>>,
    location: Path,
    last_etag: RwLock<Option<String>>,
}

impl SupergraphStorageFetcher {
    async fn fetch_supergraph(&self) -> Result<Option<String>, LoadSupergraphError> {
        let etag = {
            let read_guard = self.last_etag.read().await;

            (*read_guard).clone()
        };

        let result = self
            .storage
            .get_if_none_changed(&self.location, etag)
            .await?;

        match result {
            StorageGetResult::NotModified => Ok(None),
            StorageGetResult::Modified { contents, etag } => {
                *self.last_etag.write().await = etag;

                Ok(Some(contents))
            }
        }
    }
}

impl SupergraphStorageLoader {
    pub fn try_new(
        fetcher: Arc<Box<dyn StorageRuntime>>,
        location: String,
        poll_interval: Option<Duration>,
    ) -> Result<Box<Self>, LoadSupergraphError> {
        Ok(Box::new(Self {
            fetcher: SupergraphStorageFetcher {
                storage: fetcher,
                location: Path::from(location),
                last_etag: RwLock::new(None),
            },
            poll_interval,
        }))
    }
}

#[async_trait]
impl SupergraphLoader for SupergraphStorageLoader {
    async fn load(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError> {
        let fetcher_result = self.fetcher.fetch_supergraph().await;

        match fetcher_result {
            // If there was an error fetching the supergraph, propagate it
            Err(err) => {
                error!("Error fetching supergraph from storage: {}", err);
                Err(err)
            }
            // If the supergraph has not changed, return Unchanged
            Ok(None) => Ok(ReloadSupergraphResult::Unchanged),
            // If there is a new supergraph SDL, return it
            Ok(Some(sdl)) => Ok(ReloadSupergraphResult::Changed { new_sdl: sdl }),
        }
    }

    fn reload_interval(&self) -> Option<&std::time::Duration> {
        self.poll_interval.as_ref()
    }
}
