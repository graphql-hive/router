use std::collections::VecDeque;

use query_planner::planner::plan_nodes::FlattenNodePathSegment;

use crate::{
    introspection::schema::SchemaMetadata, response::value::Value,
    utils::consts::TYPENAME_FIELD_NAME,
};

pub fn traverse_and_callback<'a, E, Callback>(
    current_data: &'a Value<'a>,
    remaining_path: &'a [FlattenNodePathSegment],
    schema_metadata: &'a SchemaMetadata,
    current_indexes: VecDeque<usize>,
    callback: &mut Callback,
) -> Result<(), E>
where
    Callback: FnMut(&'a Value<'a>, VecDeque<usize>) -> Result<(), E>,
{
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            for (index, item) in arr.iter().enumerate() {
                let mut new_indexes = current_indexes.clone();
                new_indexes.push_back(index);
                callback(item, new_indexes)?;
            }
        } else {
            callback(current_data, current_indexes)?;
        }
        return Ok(());
    }

    match &remaining_path[0] {
        FlattenNodePathSegment::List => {
            if let Value::Array(arr) = current_data {
                let rest_of_path = &remaining_path[1..];
                for (index, item) in arr.iter().enumerate() {
                    let mut new_indexes = current_indexes.clone();
                    new_indexes.push_back(index);
                    traverse_and_callback(
                        item,
                        rest_of_path,
                        schema_metadata,
                        new_indexes,
                        callback,
                    )?;
                }
            }
        }
        FlattenNodePathSegment::Field(field_name) => {
            if let Value::Object(map) = current_data {
                if let Ok(idx) = map.binary_search_by_key(&field_name.as_str(), |(k, _)| k) {
                    let (_, next_data) = &map[idx];
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback(
                        next_data,
                        rest_of_path,
                        schema_metadata,
                        current_indexes,
                        callback,
                    )?;
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
                    traverse_and_callback(
                        current_data,
                        rest_of_path,
                        schema_metadata,
                        current_indexes,
                        callback,
                    )?;
                }
            } else if let Value::Array(arr) = current_data {
                for (index, item) in arr.iter().enumerate() {
                    let mut new_indexes = current_indexes.clone();
                    new_indexes.push_back(index);
                    traverse_and_callback(
                        item,
                        remaining_path,
                        schema_metadata,
                        new_indexes,
                        callback,
                    )?;
                }
            }
        }
    }

    Ok(())
}
