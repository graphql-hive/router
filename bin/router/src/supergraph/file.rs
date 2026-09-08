use std::io::ErrorKind;
use std::time::Duration;

use crate::config::primitives::file_path::FilePath;
use crate::telemetry::logging::targets;
use async_trait::async_trait;
use tokio::{fs, sync::RwLock};
use tracing::{debug, trace};

use crate::supergraph::base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader};

/// Maximum number of retry attempts for a transient read failure.
const MAX_TRANSIENT_RETRIES: usize = 3;

/// A read of ConfigMap (K8s) that lands in that tiny window can fail with not found error
/// or stale error on linux.
fn is_transient_read_error(err: &std::io::Error) -> bool {
    // ESTALE has no dedicated `ErrorKind` variant, so match its raw errno (116 on Linux).
    matches!(err.kind(), ErrorKind::NotFound) || err.raw_os_error() == Some(116)
}

#[derive(Debug, thiserror::Error)]
pub enum FileSupergraphError {
    #[error("Failed to read supergraph file: {0}")]
    ReadFileError(#[from] std::io::Error),
    #[error("Supergraph file path is missing. Please provide it via 'SUPERGRAPH_FILE_PATH' environment variable or under 'supergraph.path' in the configuration.")]
    MissingSupergraphFilePath,
}

pub struct SupergraphFileLoader {
    file_path: FilePath,
    poll_interval: Option<Duration>,
    modified_time: RwLock<Option<std::time::SystemTime>>,
}

impl SupergraphFileLoader {
    async fn load_with_polling(&self) -> Result<ReloadSupergraphResult, FileSupergraphError> {
        // Retry transient failures caused by a ConfigMap atomic swap racing with our
        // read (see `is_transient_read_error`). Retries settle well within one poll
        // interval, so a genuine `NotFound` still surfaces as an error after a few tries.
        let mut attempt: usize = 0;
        loop {
            match self.try_load_with_polling().await {
                Err(FileSupergraphError::ReadFileError(err))
                    if is_transient_read_error(&err) && attempt < MAX_TRANSIENT_RETRIES =>
                {
                    attempt += 1;

                    debug!(
                        target: targets::SUPERGRAPH,
                        path = ?self.file_path.absolute,
                        attempt,
                        error = ?err,
                        "transient read during supergraph reload, retrying",
                    );

                    tokio::time::sleep(Duration::from_millis(50 * attempt as u64)).await;
                }
                other => return other,
            }
        }
    }

    async fn try_load_with_polling(&self) -> Result<ReloadSupergraphResult, FileSupergraphError> {
        let file_metadata = fs::metadata(&self.file_path.absolute).await?;
        let current_time = file_metadata.modified()?;
        let mut modified_time = self.modified_time.write().await;

        if modified_time.is_none_or(|previous| current_time != previous) {
            let content = fs::read_to_string(&self.file_path.absolute).await?;
            *modified_time = Some(current_time);

            Ok(ReloadSupergraphResult::Changed { new_sdl: content })
        } else {
            Ok(ReloadSupergraphResult::Unchanged)
        }
    }

    async fn load_without_polling(&self) -> Result<ReloadSupergraphResult, FileSupergraphError> {
        let content = fs::read_to_string(&self.file_path.absolute).await?;

        Ok(ReloadSupergraphResult::Changed { new_sdl: content })
    }
}

#[async_trait]
impl SupergraphLoader for SupergraphFileLoader {
    async fn load(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError> {
        let result = if self.poll_interval.is_some() {
            debug!(
                target: targets::SUPERGRAPH,
                path = ?self.file_path.absolute,
                interval_ms = ?self.poll_interval.as_ref().map(|i| i.as_millis()),
                "Loading supergraph from file, and checking metadata for polling",
            );

            self.load_with_polling().await
        } else {
            debug!(
              target: targets::SUPERGRAPH,
              path = ?self.file_path.absolute,
                "Loading supergraph from file",
            );

            self.load_without_polling().await
        };

        trace!(
          target: targets::SUPERGRAPH,
          path = ?self.file_path.absolute,
          result = ?result,
          "Supergraph loaded from file",
        );

        Ok(result?)
    }

    fn reload_interval(&self) -> Option<std::time::Duration> {
        self.poll_interval
    }
}

impl SupergraphFileLoader {
    pub fn new(
        file_path: &FilePath,
        poll_interval: Option<Duration>,
    ) -> Result<Box<Self>, FileSupergraphError> {
        debug!(
          target: targets::SUPERGRAPH,
          path = ?file_path.absolute,
          "Creating supergraph source from file",
        );

        Ok(Box::new(Self {
            file_path: file_path.clone(),
            poll_interval,
            modified_time: RwLock::new(None),
        }))
    }
}
