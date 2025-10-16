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
    /// Loads a supergraph from Hive Console CDN.
    ///
    #[serde(rename = "hive")]
    HiveConsole { endpoint: String, key: String },
}

impl Default for SupergraphSource {
    fn default() -> Self {
        SupergraphSource::File {
            path: FilePath::new_from_relative("supergraph.graphql")
                .expect("failed to resolve local path for supergraph file source"),
        }
    }
}
