use sonic_rs::{JsonContainerTrait, JsonValueMutTrait, JsonValueTrait, Value};
use tracing::instrument;

// Deeply merges two serde_json::Values (mutates target in place)
pub fn deep_merge(target: &mut Value, source: Value) {
    // 1. Source is Null: Do nothing
    if source.is_null() {
        return {};
    }

    // 2. Both are Objects: Merge recursively
    if target.is_object() && source.is_object() {
        return deep_merge_objects(
            target.as_object_mut().unwrap(),
            source.as_object().unwrap().to_owned(),
        );
    }

    // 3. Both are Arrays of same length(?): Merge elements
    if target.is_array() && source.is_array() {
        let target_arr = target.as_array_mut().unwrap();
        let source_arr = source.as_array().unwrap();
        for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
            deep_merge(target_val, source_val.to_owned());
        }
    }

    // 4. Fallback: Source is not Null, and cases 2 & 3 didn't match. Replace target with source.
    *target = source;
}

#[instrument(
    skip(target_map, source_map),
    fields(
        target_type = %target_map.get(&"__typename").map_or("unknown", |v| v.as_str().unwrap_or("unknown")),
        source_type = %source_map.get(&"__typename").map_or("unknown", |v| v.as_str().unwrap_or("unknown"))
    ),
    level = "trace"
)]
pub fn deep_merge_objects(target_map: &mut sonic_rs::Object, source_map: sonic_rs::Object) {
    if target_map.is_empty() {
        // If target is empty, just replace it with source
        *target_map = source_map;
        return;
    }
    if source_map.is_empty() {
        // If source is empty, do nothing (target remains unchanged)
        return;
    }
    for (key, source_val) in source_map.iter() {
        if let Some(target_val) = target_map.get_mut(&key) {
            // If key exists in target, merge recursively
            deep_merge(target_val, source_val.to_owned());
        } else {
            // If key does not exist in target, insert it
            target_map.insert(key, source_val.to_owned());
        }
    }
}
