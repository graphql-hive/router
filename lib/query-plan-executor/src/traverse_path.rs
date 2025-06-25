use serde_json::Value;
use tracing::{instrument, trace};

use crate::deep_merge::DeepMerge;

#[derive(Debug, Clone)]
pub enum TraversedPathSegment {
    Index(usize),
    Field(String),
}

pub trait SetPathValue {
    fn set_path_value(&mut self, path: &[TraversedPathSegment], value: Value);
}

impl SetPathValue for Value {
    fn set_path_value(&mut self, path: &[TraversedPathSegment], value: Value) {
        let current_segment = &path[0];
        let remaining_path = path.get(1..).unwrap_or(&[]);
        if self.is_null() {
            // If the current value is null, we need to create a new structure
            match current_segment {
                TraversedPathSegment::Index(index) => {
                    *self = Value::Array(vec![Value::Null; *index + 1]);
                }
                TraversedPathSegment::Field(_field) => {
                    *self = Value::Object(serde_json::Map::with_capacity(1));
                }
            }
        }
        if remaining_path.is_empty() {
            match self {
                Value::Array(array) => {
                    // If the current value is an array, we can set the value directly
                    if let TraversedPathSegment::Index(index) = current_segment {
                        if *index >= array.len() {
                            // Extend the array with nulls if necessary
                            array.resize(*index + 1, Value::Null);
                        }
                        array[*index].deep_merge(value);
                    } else {
                        trace!(
                            "Cannot set value at path segment {:?} in current data",
                            current_segment
                        );
                    }
                }
                Value::Object(map) => {
                    // If the current value is an object, we can set the value directly
                    if let TraversedPathSegment::Field(field) = current_segment {
                        if let Some(existing_value) = map.get_mut(field) {
                            // If the field exists, merge the value
                            existing_value.deep_merge(value);
                        } else {
                            // If the field does not exist, insert it
                            map.insert(field.to_string(), value);
                        }
                    } else {
                        trace!(
                            "Cannot set value at path segment {:?} in current data",
                            current_segment
                        );
                    }
                }
                _ => {
                    trace!(
                        "Cannot set value at path segment {:?} in current data",
                        current_segment
                    );
                }
            }
            return;
        }
        match (self, current_segment) {
            (Value::Array(array), TraversedPathSegment::Index(index)) => {
                if *index >= array.len() {
                    // Extend the array with nulls if necessary
                    array.resize(index + 1, Value::Null);
                }
                array[*index].set_path_value(remaining_path, value);
            }
            (Value::Object(map), TraversedPathSegment::Field(field)) => {
                if !map.contains_key(field) {
                    // If the field does not exist, create it with a null value
                    map.insert(field.to_string(), Value::Null);
                }
                map.get_mut(field)
                    .unwrap()
                    .set_path_value(remaining_path, value);
            }
            (_, _) => {
                // If the current value is not compatible with the path, we can't set the value
                println!(
                    "Cannot set value at path segment {:?} in current data",
                    current_segment
                );
            }
        }
    }
}

/// Recursively traverses the data according to the path segments,
/// handling '@' for array iteration, and collects the final values.current_data.to_vec()
#[instrument(level = "trace", skip_all, fields(
    current_type = ?current_data,
    remaining_path = ?remaining_path
))]
pub fn traverse_path<'a, Callback>(
    current_data: &'a Value,
    current_path: Vec<TraversedPathSegment>,
    remaining_path: &[&str],
    callback: &mut Callback,
) where
    Callback: FnMut(Vec<TraversedPathSegment>, &'a Value),
{
    if current_data.is_null() {
        // If current_data is null, we can't traverse further
        tracing::warn!("Encountered null value at path: {:?}", current_path);
        return;
    }
    if remaining_path.is_empty() {
        return match current_data {
            Value::Null => {
                // tracing::warn!("Reached end of path with null value at path: {}", current_path.join("."));
            }
            Value::Array(arr) => {
                for (index, item) in arr.iter().enumerate() {
                    let mut path_with_index = current_path.clone();
                    path_with_index.push(TraversedPathSegment::Index(index));
                    callback(path_with_index, item);
                }
            }
            _ => {
                // Call the callback with the current path and data
                callback(current_path, current_data);
            }
        };
    }

    let next_segment = remaining_path[0];
    let next_remaining_path = &remaining_path[1..];

    if next_segment == "@" {
        // Handle array iteration
        if let Value::Array(array) = current_data {
            for (index, item) in array.iter().enumerate() {
                let mut path_with_index = current_path.clone();
                path_with_index.push(TraversedPathSegment::Index(index));
                traverse_path(item, path_with_index, next_remaining_path, callback);
            }
        } else {
            // If current_data is not an array, we can't iterate
            tracing::warn!(
                "Expected an array at path segment '{}', found: {:?}",
                next_segment,
                current_data
            );
        }
    } else {
        // Handle object field access
        if let Value::Object(map) = current_data {
            if let Some(next_value) = map.get(next_segment) {
                let mut path_with_index = current_path.clone();
                path_with_index.push(TraversedPathSegment::Field(next_segment.to_string()));
                traverse_path(next_value, path_with_index, next_remaining_path, callback);
            } else {
                // tracing::warn!("Field '{}' not found in object at path segment '{}'", next_segment, current_path.join("."));
            }
        } else {
            // If current_data is not an object, we can't access fields
            tracing::warn!(
                "Expected an object at path segment '{}', found: {:?}",
                next_segment,
                current_data
            );
        }
    }
}
