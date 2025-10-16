use std::path::Path;

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
            self.file_path.absolute
        );
        let content = tokio::fs::read_to_string(&self.file_path.absolute).await?;
        trace!(
            "Supergraph loaded from file path: '{}', content: {}",
            self.file_path.absolute,
            content
        );

        self.current = Some(content);
        Ok(())
    }

    fn init_watcher(&self) -> bool {
        let absolute_path = Path::new(&self.file_path.absolute);

        debug!("okok");
        debug!(
            "creating a watcher for file path: {}",
            absolute_path.to_string_lossy()
        );

        // let mut watcher = RecommendedWatcher::new(
        //     move |res: notify::Result<Event>| match res {
        //         Ok(event) => {
        //             info!("got event");

        //             if event.kind.is_modify() {
        //                 info!("modify event");

        //                 let _ = tx.send(SupergraphManagerMessage::SupergraphChanged);
        //             }
        //         }
        //         Err(e) => {
        //             error!("Error watching file: {}", e);
        //         }
        //     },
        //     notify::Config::default(),
        // )
        // .expect("failed to create watcher");

        // watcher
        //     .watch(&absolute_path, notify::RecursiveMode::NonRecursive)
        //     .expect("failed to watch file");

        true
    }

    fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }
}

impl SupergraphFileLoader {
    pub async fn new(file_path: &FilePath) -> Result<Box<Self>, LoadSupergraphError> {
        debug!(
            "Creating supergraph source from file path: '{}'",
            file_path.absolute
        );

        Ok(Box::new(Self {
            file_path: file_path.clone(),
            current: None,
        }))
    }
}
