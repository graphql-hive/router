use query_planner::planner::plan_nodes::FlattenNodePathSegment;

use crate::{
    introspection::schema::SchemaMetadata, response::value::Value,
    utils::consts::TYPENAME_FIELD_NAME,
};

pub fn traverse_and_callback_mut<'a, Callback>(
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
                    traverse_and_callback_mut(item, rest_of_path, schema_metadata, callback);
                }
            }
        }
        FlattenNodePathSegment::Field(field_name) => {
            // If the key is Field, we expect current_data to be an object
            if let Value::Object(map) = current_data {
                if let Ok(idx) = map.binary_search_by_key(&field_name.as_str(), |(k, _)| k) {
                    let (_, next_data) = map.get_mut(idx).unwrap();
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback_mut(next_data, rest_of_path, schema_metadata, callback);
                }
            }
        }
        FlattenNodePathSegment::Cast(type_condition) => {
            // If the key is Cast, we expect current_data to be an object or an array
            if let Value::Object(obj) = current_data {
                let type_name = obj
                    .binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| k)
                    .ok()
                    .and_then(|idx| obj[idx].1.as_str())
                    .unwrap_or(type_condition);
                if schema_metadata
                    .possible_types
                    .entity_satisfies_type_condition(type_name, type_condition)
                {
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback_mut(
                        current_data,
                        rest_of_path,
                        schema_metadata,
                        callback,
                    );
                }
            } else if let Value::Array(arr) = current_data {
                // If the current data is an array, we need to check each item
                for item in arr.iter_mut() {
                    traverse_and_callback_mut(item, remaining_path, schema_metadata, callback);
                }
            }
        }
    }
}

pub fn traverse_and_callback<'a, E, Callback>(
    current_data: &'a Value<'a>,
    remaining_path: &'a [FlattenNodePathSegment],
    schema_metadata: &'a SchemaMetadata,
    callback: &mut Callback,
) -> Result<(), E>
where
    Callback: FnMut(&'a Value<'a>) -> Result<(), E>,
{
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            for item in arr.iter() {
                callback(item)?;
            }
        } else {
            callback(current_data)?;
        }
        return Ok(());
    }

    match &remaining_path[0] {
        FlattenNodePathSegment::List => {
            if let Value::Array(arr) = current_data {
                let rest_of_path = &remaining_path[1..];
                for item in arr.iter() {
                    traverse_and_callback(item, rest_of_path, schema_metadata, callback)?;
                }
            }
        }
        FlattenNodePathSegment::Field(field_name) => {
            if let Value::Object(map) = current_data {
                if let Ok(idx) = map.binary_search_by_key(&field_name.as_str(), |(k, _)| k) {
                    let (_, next_data) = &map[idx];
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback(next_data, rest_of_path, schema_metadata, callback)?;
                }
            }
        }
        FlattenNodePathSegment::Cast(type_condition) => {
            if let Value::Object(obj) = current_data {
                let type_name = obj
                    .binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| k)
                    .ok()
                    .and_then(|idx| obj[idx].1.as_str())
                    .unwrap_or(type_condition);
                if schema_metadata
                    .possible_types
                    .entity_satisfies_type_condition(type_name, type_condition)
                {
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback(current_data, rest_of_path, schema_metadata, callback)?;
                }
            } else if let Value::Array(arr) = current_data {
                for item in arr.iter() {
                    traverse_and_callback(item, remaining_path, schema_metadata, callback)?;
                }
            }
        }
    }

    Ok(())
}
