use query_planner::planner::plan_nodes::KeyRenamer;
use serde_json::Value;

use crate::{
    fetch_rewrites::ApplyFetchRewrite,
    schema_metadata::{EntitySatisfiesTypeCondition, SchemaMetadata},
    TYPENAME_FIELD,
};

impl ApplyFetchRewrite for KeyRenamer {
    fn apply(&self, schema_metadata: &SchemaMetadata, value: &mut Value) {
        self.apply_path(schema_metadata, value, &self.path)
    }
    // Applies key rename operation on a Value (mutably)
    fn apply_path(&self, schema_metadata: &SchemaMetadata, value: &mut Value, path: &[String]) {
        let current_segment = &path[0];
        let remaining_path = &path[1..];

        match value {
            Value::Array(arr) => {
                for item in arr {
                    self.apply_path(schema_metadata, item, path);
                }
            }
            Value::Object(obj) => {
                let type_condition = current_segment.strip_prefix("... on ");
                match type_condition {
                    Some(type_condition) => {
                        let type_name = match obj.get(TYPENAME_FIELD) {
                            Some(Value::String(type_name)) => type_name,
                            _ => type_condition, // Default to type_condition if not found
                        };
                        if schema_metadata
                            .entity_satisfies_type_condition(type_name, type_condition)
                        {
                            self.apply_path(schema_metadata, value, remaining_path)
                        }
                    }
                    _ => {
                        if remaining_path.is_empty() {
                            if *current_segment != self.rename_key_to {
                                if let Some(val) = obj.remove(current_segment) {
                                    obj.insert(self.rename_key_to.to_string(), val);
                                }
                            }
                        } else if let Some(next_value) = obj.get_mut(current_segment) {
                            self.apply_path(schema_metadata, next_value, remaining_path)
                        }
                    }
                }
            }
            _ => (),
        }
    }
}
