use async_trait::async_trait;
use hive_router_config::primitives::file_path::FilePath;
use tracing::{debug, trace};

use crate::supergraph::base::{LoadSupergraphError, SupergraphLoader};

pub struct SupergraphFileLoader {
    file_path: FilePath,
    current: Option<String>,
}

#[async_trait]
impl SupergraphLoader for SupergraphFileLoader {
    async fn reload(&mut self) -> Result<(), LoadSupergraphError> {
        debug!(
            "Reloading supergraph from file path: '{}'",
            self.file_path.0
        );
        let content = tokio::fs::read_to_string(&self.file_path.0).await?;
        trace!(
            "Supergraph loaded from file path: '{}', content: {}",
            self.file_path.0,
            content
        );

        self.current = Some(content);
        Ok(())
    }

    fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }
}

impl SupergraphFileLoader {
    pub async fn new(file_path: &FilePath) -> Result<Box<Self>, LoadSupergraphError> {
        debug!(
            "Creating supergraph source from file path: '{}'",
            file_path.0
        );

        Ok(Box::new(Self {
            file_path: file_path.clone(),
            current: None,
        }))
    }
}
