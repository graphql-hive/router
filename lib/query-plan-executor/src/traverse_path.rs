use serde_json::Value;
use tracing::instrument;

#[instrument(level = "trace", skip_all, fields(
    current_type = ?current_data,
    remaining_path = ?remaining_path
))]
pub fn traverse_path<'a, Callback>(
    current_data: &'a mut Value,
    current_path: Vec<String>,
    remaining_path: &[&str],
    callback: &mut Callback,
) where
    Callback: FnMut(Vec<String>, &'a mut Value),
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
                for (index, item) in arr.iter_mut().enumerate() {
                    let mut path_with_index = current_path.clone();
                    path_with_index.push(index.to_string());
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
            for (index, item) in array.iter_mut().enumerate() {
                let mut path_with_index = current_path.clone();
                path_with_index.push(index.to_string());
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
            if let Some(next_value) = map.get_mut(next_segment) {
                let mut path_with_field = current_path.clone();
                path_with_field.push(next_segment.to_string());
                traverse_path(next_value, path_with_field, next_remaining_path, callback);
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
