use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::file_path::FilePath;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, tag = "source")]
pub enum SupergraphSource {
    /// Loads a supergraph from the filesystem.
    /// The path can be either absolute or relative to the router's working directory.
    #[serde(rename = "file")]
    File { path: FilePath },
}

impl Default for SupergraphSource {
    fn default() -> Self {
        SupergraphSource::File {
            path: FilePath::new_from_relative("supergraph.graphql")
                .expect("failed to resolve local path for supergraph file source"),
        }
    }
}

impl SupergraphSource {
    pub async fn load(&self) -> Result<String, Box<dyn std::error::Error>> {
        match self {
            SupergraphSource::File { path } => {
                let supergraph_sdl = std::fs::read_to_string(&path.absolute).map_err(|e| {
                    std::io::Error::new(
                        e.kind(),
                        format!("Failed to read supergraph file '{}': {}", path.absolute, e),
                    )
                })?;
                Ok(supergraph_sdl)
            }
        }
    }
}
