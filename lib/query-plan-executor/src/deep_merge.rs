use query_planner::ast::{selection_item::SelectionItem, selection_set::SelectionSet};
use serde_json::Value;
use tracing::{instrument, trace};

// Deeply merges two serde_json::Values (mutates target in place)
#[instrument(level = "trace", name = "deep_merge", skip_all)]
pub fn deep_merge(target: &mut Value, source: Value, selection_set: &SelectionSet) {
    match (target, source) {
        (_, Value::Null) => {
            trace!("Source is Null, keeping target as is");
        }

        (Value::Object(target_map), Value::Object(mut source_map)) => {
            trace!(
                "Merging two objects: target_map_len={}, source_map_len={}",
                target_map.len(),
                source_map.len()
            );
            if target_map.is_empty() {
                // If target is empty, just replace it with source
                trace!("Target map is empty, replacing with source map");
                *target_map = source_map;
            } else {
                deep_merge_objects(target_map, &mut source_map, selection_set);
            }
        }

        // 3. Both are Arrays of same length(?): Merge elements
        (Value::Array(target_arr), Value::Array(source_arr)) => {
            trace!(
                "Merging two arrays: target_arr_len={}, source_arr_len={}",
                target_arr.len(),
                source_arr.len()
            );
            for (target_val, source_val) in target_arr.iter_mut().zip(source_arr) {
                deep_merge(target_val, source_val, selection_set);
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

#[instrument(
    level = "trace", 
    name = "deep_merge_objects", 
    skip_all,
    fields(target_map_len = target_map.len(), source_map_len = source_map.len(), typename= %target_map.get("typename").and_then(|a| a.as_str()).unwrap_or("unknown"))
)]
pub fn deep_merge_objects(
    target_map: &mut serde_json::Map<String, Value>,
    source_map: &mut serde_json::Map<String, Value>,
    selection_set: &SelectionSet,
) {
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
    for item in selection_set.items.iter() {
        match item {
            SelectionItem::Field(field) => {
                let field_name = &field.name;
                let response_key = field.alias.as_deref().unwrap_or(field_name);
                trace!("Processing field: {}", response_key);
                if let Some((response_key, source_val)) = source_map.remove_entry(response_key) {
                    if let Some(target_val) = target_map.get_mut(&response_key) {
                        // If key exists in target, merge recursively
                        trace!("Key '{}' exists in target, merging values", response_key);
                        deep_merge(target_val, source_val, &field.selections);
                    } else {
                        // If key does not exist in target, insert it
                        trace!(
                            "Key '{}' does not exist in target, inserting value",
                            response_key
                        );
                        target_map.insert(response_key, source_val);
                    }
                }
            }
            SelectionItem::InlineFragment(inline_fragment) => {
                deep_merge_objects(target_map, source_map, &inline_fragment.selections);
            }
            SelectionItem::FragmentSpread(_) => {
                // Not the case
                unimplemented!("Fragment spreads are not supported in deep_merge");
            }
        }
    }
}
