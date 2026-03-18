use std::time::Duration;

use async_trait::async_trait;
use hive_router_config::primitives::file_path::FilePath;
use tokio::{fs, sync::RwLock};
use tracing::{debug, info, trace};

use crate::supergraph::base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader};

pub struct SupergraphFileLoader {
    file_path: FilePath,
    poll_interval: Option<Duration>,
    modified_time: RwLock<Option<std::time::SystemTime>>,
}

impl SupergraphFileLoader {
    async fn load_with_polling(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError> {
        let file_metadata = fs::metadata(&self.file_path.absolute).await?;
        let current_time = file_metadata.modified()?;
        let mut modified_time = self.modified_time.write().await;

        if modified_time.is_none() || current_time > modified_time.unwrap() {
            let content = fs::read_to_string(&self.file_path.absolute).await?;
            *modified_time = Some(current_time);

            Ok(ReloadSupergraphResult::Changed { new_sdl: content })
        } else {
            Ok(ReloadSupergraphResult::Unchanged)
        }
    }

    async fn load_without_polling(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError> {
        let content = fs::read_to_string(&self.file_path.absolute).await?;

        Ok(ReloadSupergraphResult::Changed { new_sdl: content })
    }
}

#[async_trait]
impl SupergraphLoader for SupergraphFileLoader {
    async fn load(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError> {
        let result = if self.poll_interval.is_some() {
            debug!(
                file_path = self.file_path.absolute,
                "Loading supergraph from file (polling enabled)",
            );

            self.load_with_polling().await
        } else {
            debug!(
                file_path = self.file_path.absolute,
                "Loading supergraph from file (polling disabled)",
            );

            self.load_without_polling().await
        };

        info!(
            file_path = self.file_path.absolute,
            "Supergraph successfully loaded from a local file"
        );

        trace!(
            file_path = self.file_path.absolute,
            "Supergraph loaded from file, result: {:?}",
            result
        );

        result
    }

    fn reload_interval(&self) -> Option<&std::time::Duration> {
        self.poll_interval.as_ref()
    }
}

impl SupergraphFileLoader {
    pub fn new(
        file_path: &FilePath,
        poll_interval: Option<Duration>,
    ) -> Result<Box<Self>, LoadSupergraphError> {
        debug!(
            file_path = file_path.absolute,
            "Creating supergraph source from a file",
        );

        Ok(Box::new(Self {
            file_path: file_path.clone(),
            poll_interval,
            modified_time: RwLock::new(None),
        }))
    }
}
