use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::file_path::FilePath;

#[derive(Deserialize, Serialize, JsonSchema, Debug)]
#[serde(tag = "source")]
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
            path: "supergraph.graphql".into(),
        }
    }
}
