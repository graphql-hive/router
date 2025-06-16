use serde_json::Value;
use tracing::instrument;

// Deeply merges two serde_json::Values (mutates target in place)
pub fn deep_merge(target: &mut Value, source: Value) {
    match (target, source) {
        // 1. Source is Null: Do nothing
        (_, Value::Null) => {} // Keep target as is

        // 2. Both are Objects: Merge recursively
        (Value::Object(target_map), Value::Object(source_map)) => {
            deep_merge_objects(target_map, source_map);
        }

        // 3. Both are Arrays of same length(?): Merge elements
        (Value::Array(target_arr), Value::Array(source_arr)) => {
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
                deep_merge(target_val, source_val);
            }
        }

        // 4. Fallback: Source is not Null, and cases 2 & 3 didn't match. Replace target with source.
        (target_val, source) => {
            // source is guaranteed not Null here due to arm 1
            *target_val = source;
        }
    }
}

#[instrument(
    skip(target_map, source_map),
    fields(
        target_type = %target_map.get("__typename").map_or("unknown", |v| v.as_str().unwrap_or("unknown")),
        source_type = %source_map.get("__typename").map_or("unknown", |v| v.as_str().unwrap_or("unknown"))
    ),
    level = "trace"
)]
pub fn deep_merge_objects(
    target_map: &mut serde_json::Map<String, Value>,
    source_map: serde_json::Map<String, Value>,
) {
    if target_map.is_empty() {
        // If target is empty, just replace it with source
        *target_map = source_map;
        return;
    }
    if source_map.is_empty() {
        // If source is empty, do nothing (target remains unchanged)
        return;
    }
    for (key, source_val) in source_map {
        if let Some(target_val) = target_map.get_mut(&key) {
            // If key exists in target, merge recursively
            deep_merge(target_val, source_val);
        } else {
            // If key does not exist in target, insert it
            target_map.insert(key, source_val);
        }
    }
}
