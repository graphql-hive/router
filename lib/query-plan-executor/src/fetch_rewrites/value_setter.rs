use query_planner::planner::plan_nodes::ValueSetter;
use serde_json::Value;
use tracing::warn;

use crate::{
    fetch_rewrites::ApplyFetchRewrite,
    schema_metadata::{EntitySatisfiesTypeCondition, SchemaMetadata},
    TYPENAME_FIELD,
};

impl ApplyFetchRewrite for ValueSetter {
    fn apply(&self, schema_metadata: &SchemaMetadata, value: &mut Value) {
        self.apply_path(schema_metadata, value, &self.path)
    }

    // Applies value setting on a Value (returns a new Value)
    fn apply_path(&self, schema_metadata: &SchemaMetadata, data: &mut Value, path: &[String]) {
        if path.is_empty() {
            *data = self.set_value_to.to_owned();
            return;
        }

        match data {
            Value::Array(arr) => {
                for data in arr {
                    // Apply the path to each item in the array
                    self.apply_path(schema_metadata, data, path);
                }
            }
            Value::Object(map) => {
                let current_key = &path[0];
                let remaining_path = &path[1..];

                if let Some(type_condition) = current_key.strip_prefix("... on ") {
                    let type_name = match map.get(TYPENAME_FIELD) {
                        Some(Value::String(type_name)) => type_name,
                        _ => type_condition, // Default to type_condition if not found
                    };
                    if schema_metadata.entity_satisfies_type_condition(type_name, type_condition) {
                        self.apply_path(schema_metadata, data, remaining_path)
                    }
                } else if let Some(data) = map.get_mut(current_key) {
                    // If the key exists, apply the remaining path to its value
                    self.apply_path(schema_metadata, data, remaining_path)
                }
            }
            _ => {
                warn!(
                    "Trying to apply ValueSetter path {:?} to non-object/array type: {:?}",
                    path, data
                );
            }
        }
    }
}
