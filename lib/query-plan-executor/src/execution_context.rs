use serde_json::{Map, Value};

use crate::{executors::map::SubgraphExecutorMap, schema_metadata::SchemaMetadata};

pub struct ExecutionContext<'a> {
    pub variables: &'a Option<Map<String, Value>>,
    pub schema_metadata: &'a SchemaMetadata,
    pub subgraph_executor_map: &'a SubgraphExecutorMap,
}
