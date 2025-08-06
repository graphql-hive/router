use query_plan_executor::schema_metadata::SchemaMetadata;
use query_planner::planner::plan_nodes::FlattenNodePathSegment;

use crate::{response::value::Value, utils::consts::TYPENAME_FIELD_NAME};

pub fn traverse_and_callback<'a, Callback>(
    current_data: &mut Value<'a>,
    remaining_path: &[FlattenNodePathSegment],
    schema_metadata: &SchemaMetadata,
    callback: &mut Callback,
) where
    Callback: FnMut(&mut Value),
{
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            // If the path is empty, we call the callback on each item in the array
            // We iterate because we want the entity objects directly
            for item in arr.iter_mut() {
                callback(item);
            }
        } else {
            // If the path is empty and current_data is not an array, just call the callback
            callback(current_data);
        }
        return;
    }

    match &remaining_path[0] {
        FlattenNodePathSegment::List => {
            // If the key is List, we expect current_data to be an array
            if let Value::Array(arr) = current_data {
                let rest_of_path = &remaining_path[1..];
                for item in arr.iter_mut() {
                    traverse_and_callback(item, rest_of_path, schema_metadata, callback);
                }
            }
        }
        FlattenNodePathSegment::Field(field_name) => {
            // If the key is Field, we expect current_data to be an object
            if let Value::Object(map) = current_data {
                if let Some((_, next_data)) = map.iter_mut().find(|(key, _)| key == field_name) {
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback(next_data, rest_of_path, schema_metadata, callback);
                }
            }
        }
        FlattenNodePathSegment::Cast(type_condition) => {
            // If the key is Cast, we expect current_data to be an object or an array
            if let Value::Object(obj) = current_data {
                let type_name = obj
                    .iter()
                    .find(|(key, _)| key == &TYPENAME_FIELD_NAME)
                    .and_then(|(_, val)| val.as_str())
                    .unwrap_or(type_condition);
                if schema_metadata
                    .possible_types
                    .entity_satisfies_type_condition(type_name, type_condition)
                {
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback(current_data, rest_of_path, schema_metadata, callback);
                }
            } else if let Value::Array(arr) = current_data {
                // If the current data is an array, we need to check each item
                for item in arr.iter_mut() {
                    traverse_and_callback(item, remaining_path, schema_metadata, callback);
                }
            }
        }
    }
}

pub fn traverse_and_collect<'a>(
    current_data: &'a mut Value<'a>,
    remaining_path: &[FlattenNodePathSegment],
    schema_metadata: &SchemaMetadata,
) -> Vec<&'a mut Value<'a>> {
    let mut results = Vec::new();
    traverse_and_collect_recursive(current_data, remaining_path, schema_metadata, &mut results);
    results
}

fn traverse_and_collect_recursive<'a>(
    current_data: &'a mut Value<'a>,
    remaining_path: &[FlattenNodePathSegment],
    schema_metadata: &SchemaMetadata,
    results: &mut Vec<&'a mut Value<'a>>,
) {
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            results.extend(arr.iter_mut());
        } else {
            results.push(current_data);
        }
        return;
    }

    let rest_of_path = &remaining_path[1..];

    match &remaining_path[0] {
        FlattenNodePathSegment::List => {
            if let Value::Array(arr) = current_data {
                for item in arr.iter_mut() {
                    traverse_and_collect_recursive(item, rest_of_path, schema_metadata, results);
                }
            }
        }
        FlattenNodePathSegment::Field(field_name) => {
            if let Value::Object(map) = current_data {
                if let Some((_, next_data)) = map.iter_mut().find(|(key, _)| key == field_name) {
                    traverse_and_collect_recursive(
                        next_data,
                        rest_of_path,
                        schema_metadata,
                        results,
                    );
                }
            }
        }
        FlattenNodePathSegment::Cast(type_condition) => {
            if let Value::Object(obj) = current_data {
                let type_name = obj
                    .iter()
                    .find(|(key, _)| key == &TYPENAME_FIELD_NAME)
                    .and_then(|(_, val)| val.as_str())
                    .unwrap_or(type_condition);
                if schema_metadata
                    .possible_types
                    .entity_satisfies_type_condition(type_name, type_condition)
                {
                    traverse_and_collect_recursive(
                        current_data,
                        rest_of_path,
                        schema_metadata,
                        results,
                    );
                }
            } else if let Value::Array(arr) = current_data {
                for item in arr.iter_mut() {
                    traverse_and_collect_recursive(item, remaining_path, schema_metadata, results);
                }
            }
        }
    }
}
