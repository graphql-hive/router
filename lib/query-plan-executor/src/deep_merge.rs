use serde_json::Value;
use tracing::trace;

// Deeply merges two serde_json::Values (mutates target in place)
pub fn deep_merge(target: &mut Value, source: Value) {
    match (target, source) {
        (_, Value::Null) => {
            trace!("Source is Null, keeping target as is");
        }

        (Value::Object(target_map), Value::Object(source_map)) => {
            trace!(
                "Merging two objects: target_map_len={}, source_map_len={}",
                target_map.len(),
                source_map.len()
            );
            deep_merge_objects(target_map, source_map);
        }

        // 3. Both are Arrays of same length(?): Merge elements
        (Value::Array(target_arr), Value::Array(source_arr)) => {
            trace!(
                "Merging two arrays: target_arr_len={}, source_arr_len={}",
                target_arr.len(),
                source_arr.len()
            );
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
                deep_merge(target_val, source_val);
            }
        }

        // 4. Fallback: Source is not Null, and cases 2 & 3 didn't match. Replace target with source.
        (target_val, source) => {
            trace!(
                "Replacing target value with source value: target_val={}, source={}",
                target_val,
                source
            );
            // source is guaranteed not Null here due to arm 1
            *target_val = source;
        }
    }
}

pub fn deep_merge_objects(
    target_map: &mut serde_json::Map<String, Value>,
    source_map: serde_json::Map<String, Value>,
) {
    if target_map.is_empty() {
        // If target is empty, just replace it with source
        trace!("Target map is empty, replacing with source map");
        *target_map = source_map;
        return;
    }
    if source_map.is_empty() {
        // If source is empty, do nothing (target remains unchanged)
        trace!("Source map is empty, keeping target map as is");
        return;
    }
    trace!(
        "Deep merging objects: target_map_len={}, source_map_len={}",
        target_map.len(),
        source_map.len()
    );
    for (key, source_val) in source_map {
        if let Some(target_val) = target_map.get_mut(&key) {
            // If key exists in target, merge recursively
            trace!("Key '{}' exists in target, merging values", key);
            deep_merge(target_val, source_val);
        } else {
            trace!("Key '{}' does not exist in target, inserting value", key);
            // If key does not exist in target, insert it
            target_map.insert(key, source_val);
        }
    }
}
