use serde_json::Value;

// Deeply merges two serde_json::Values (mutates target in place)
pub fn deep_merge(target: &mut Value, source: Value) {
    match (target, source) {
        // 1. Source is Null: Do nothing
        (_, Value::Null) => {} // Keep target as is

        // 2. Both are Objects: Merge recursively
        (Value::Object(target_map), Value::Object(source_map)) => {
            for (key, source_val) in source_map {
                // Optimization: If source_val is Null, we could skip, but deep_merge handles it.
                let target_entry = target_map.entry(key).or_insert(Value::Null);
                deep_merge(target_entry, source_val);
            }
        }

        // 3. Both are Arrays of same length: Merge elements
        (Value::Array(target_arr), Value::Array(source_arr))
            if target_arr.len() == source_arr.len() =>
        {
            for (t, s) in target_arr.iter_mut().zip(source_arr.into_iter()) {
                // Recurse for elements. If s is Null, the recursive call handles it.
                deep_merge(t, s);
            }
        }

        // 4. Fallback: Source is not Null, and cases 2 & 3 didn't match. Replace target with source.
        (target_val, source) => {
            // source is guaranteed not Null here due to arm 1
            *target_val = source;
        }
    }
}

pub fn deep_merge_objects(
    target: &mut serde_json::Map<String, Value>,
    source: serde_json::Map<String, Value>,
) {
    if target.is_empty() {
        // If target is empty, just replace it with source
        *target = source;
        return;
    }
    for (key, source_val) in source {
        if let Some(target_val) = target.get_mut(&key) {
            // If the key exists in target, merge it
            deep_merge(target_val, source_val);
        } else {
            // If the key does not exist, insert it
            target.insert(key, source_val);
        }
    }
}
