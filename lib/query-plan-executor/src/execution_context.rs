use std::collections::BTreeMap;

use serde_json::Value;

use crate::{executors::map::SubgraphExecutorMap, schema_metadata::SchemaMetadata};

pub struct ExecutionContext<'a> {
    pub variables: &'a Option<BTreeMap<String, Value>>,
    pub schema_metadata: &'a SchemaMetadata,
    pub subgraph_executor_map: &'a SubgraphExecutorMap,
}
