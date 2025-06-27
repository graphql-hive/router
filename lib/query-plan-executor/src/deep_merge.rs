use sonic_rs::{JsonContainerTrait, JsonValueMutTrait, JsonValueTrait, Value};
use tracing::{instrument, trace};

// Deeply merges two serde_json::Values (mutates target in place)
#[instrument(level = "trace", name = "deep_merge", skip_all)]
pub fn deep_merge(target: &mut Value, source: Value) {
    if source.is_null() {
        trace!("Source is Null, keeping target as is");
        return;
    }

    if target.is_object() && source.is_object() {
        return deep_merge_objects(
            target.as_object_mut().unwrap(),
            source.as_object().unwrap().to_owned(),
        );
    }

    if target.is_array() && source.is_array() {
        let target_arr = target.as_array_mut().unwrap();
        let source_arr = source.as_array().unwrap();
        for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
            deep_merge(target_val, source_val.to_owned());
        }
        return;
    }

    *target = source;
}

pub fn deep_merge_objects(target_map: &mut sonic_rs::Object, source_map: sonic_rs::Object) {
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
    for (key, source_val) in source_map.iter() {
        if let Some(target_val) = target_map.get_mut(&key) {
            // If key exists in target, merge recursively
            trace!("Key '{}' exists in target, merging values", key);
            deep_merge(target_val, source_val.to_owned());
        } else {
            trace!("Key '{}' does not exist in target, inserting value", key);
            // If key does not exist in target, insert it
            target_map.insert(key, source_val.to_owned());
        }
    }
}
