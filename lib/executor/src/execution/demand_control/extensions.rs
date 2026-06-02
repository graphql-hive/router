use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde()]
pub struct DemandControlCostMetadataExtensions {
    pub estimated: u64,
    pub max: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<u64>,
}
