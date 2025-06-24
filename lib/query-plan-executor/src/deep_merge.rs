use serde_json::Value;
use tracing::instrument;

// Deeply merges two serde_json::Values (mutates target in place)
#[instrument(level = "trace", name = "deep_merge", skip_all)]
pub fn deep_merge(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Object(target_map), Value::Object(source_map)) => {
            deep_merge_objects(target_map, source_map);
        }
        (Value::Array(target_arr), Value::Array(source_arr)) => {
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
                deep_merge(target_val, source_val);
            }
        }
        (target, source) => {
            *target = source;
        }
    }
}

#[instrument(
    level = "trace",
    name = "deep_merge_objects",
    skip_all,
    fields(target_map_len = target_map.len(), source_map_len = source_map.len(), typename= %target_map.get("typename").and_then(|a| a.as_str()).unwrap_or("unknown"))
)]
pub fn deep_merge_objects(
    target_map: &mut serde_json::Map<String, Value>,
    source_map: serde_json::Map<String, Value>,
) {
    for (key, source_val) in source_map {
        let target_val = target_map.entry(key).or_insert(Value::Null);
        deep_merge(target_val, source_val);
    }
}
