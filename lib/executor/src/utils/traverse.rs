use hive_router_query_planner::planner::plan_nodes::FlattenNodePathSegment;

use crate::{
    introspection::schema::SchemaMetadata,
    response::{graphql_error::GraphQLErrorPath, value::Value},
    utils::consts::TYPENAME_FIELD_NAME,
};

pub fn traverse_and_callback_mut<'a, Callback>(
    current_data: &mut Value<'a>,
    remaining_path: &[FlattenNodePathSegment],
    schema_metadata: &SchemaMetadata,
    current_error_path: Option<GraphQLErrorPath>,
    callback: &mut Callback,
) where
    Callback: FnMut(&mut Value<'a>, Option<GraphQLErrorPath>),
{
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            // If the path is empty, we call the callback on each item in the array
            // We iterate because we want the entity objects directly
            for (index, item) in arr.iter_mut().enumerate() {
                let current_error_path_for_index = current_error_path
                    .as_ref()
                    .map(|current_error_path| current_error_path.concat_index(index));
                callback(item, current_error_path_for_index);
            }
        } else {
            // If the path is empty and current_data is not an array, just call the callback
            callback(current_data, current_error_path);
        }
        return;
    }

    match &remaining_path[0] {
        FlattenNodePathSegment::List => {
            // If the key is List, we expect current_data to be an array
            if let Value::Array(arr) = current_data {
                let rest_of_path = &remaining_path[1..];
                for (index, item) in arr.iter_mut().enumerate() {
                    let current_error_path_for_index = current_error_path
                        .as_ref()
                        .map(|current_error_path| current_error_path.concat_index(index));
                    traverse_and_callback_mut(
                        item,
                        rest_of_path,
                        schema_metadata,
                        current_error_path_for_index,
                        callback,
                    );
                }
            }
        }
        FlattenNodePathSegment::Field(field_name) => {
            // If the key is Field, we expect current_data to be an object
            if let Value::Object(map) = current_data {
                if let Ok(idx) = map.binary_search_by_key(&field_name.as_str(), |(k, _)| k) {
                    let (_, next_data) = map.get_mut(idx).unwrap();
                    let rest_of_path = &remaining_path[1..];
                    let current_error_path_for_field =
                        current_error_path.map(|current_error_path| {
                            current_error_path.concat_str(field_name.clone())
                        });
                    traverse_and_callback_mut(
                        next_data,
                        rest_of_path,
                        schema_metadata,
                        current_error_path_for_field,
                        callback,
                    );
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
                        current_error_path,
                        callback,
                    );
                }
            } else if let Value::Array(arr) = current_data {
                // If the current data is an array, we need to check each item
                for (index, item) in arr.iter_mut().enumerate() {
                    let current_error_path_for_index = current_error_path
                        .as_ref()
                        .map(|current_error_path| current_error_path.concat_index(index));
                    traverse_and_callback_mut(
                        item,
                        remaining_path,
                        schema_metadata,
                        current_error_path_for_index,
                        callback,
                    );
                }
            }
        }
    }
}

pub fn traverse_and_callback<'a, Callback>(
    current_data: &'a Value<'a>,
    remaining_path: &'a [FlattenNodePathSegment],
    schema_metadata: &'a SchemaMetadata,
    callback: &mut Callback,
) where
    Callback: FnMut(&'a Value<'a>),
{
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            for item in arr.iter() {
                callback(item);
            }
        } else {
            callback(current_data);
        }
        return;
    }

    match &remaining_path[0] {
        FlattenNodePathSegment::List => {
            if let Value::Array(arr) = current_data {
                let rest_of_path = &remaining_path[1..];
                for item in arr.iter() {
                    traverse_and_callback(item, rest_of_path, schema_metadata, callback);
                }
            }
        }
        FlattenNodePathSegment::Field(field_name) => {
            if let Value::Object(map) = current_data {
                if let Ok(idx) = map.binary_search_by_key(&field_name.as_str(), |(k, _)| k) {
                    let (_, next_data) = &map[idx];
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback(next_data, rest_of_path, schema_metadata, callback);
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
                    traverse_and_callback(current_data, rest_of_path, schema_metadata, callback);
                }
            } else if let Value::Array(arr) = current_data {
                for item in arr.iter() {
                    traverse_and_callback(item, remaining_path, schema_metadata, callback);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use hive_router_query_planner::planner::plan_nodes::FlattenNodePathSegment;

    use crate::{
        introspection::schema::SchemaMetadata,
        response::{
            graphql_error::{GraphQLErrorPath, GraphQLErrorPathSegment},
            value::Value,
        },
    };

    #[test]
    /**
     * Collect error paths for each item in a list at one level
     * E.g. for data { items: [ {...}, {...} ] } and path ["items", List]
     * we should collect paths ["items", 0] and ["items", 1]
     */
    fn test_collect_error_paths_one_level() {
        let mut data = Value::Object(vec![(
            "items",
            Value::Array(vec![
                Value::Object(vec![("id", Value::String("1".into()))]),
                Value::Object(vec![("id", Value::String("2".into()))]),
            ]),
        )]);
        let path = vec![
            FlattenNodePathSegment::Field("items".into()),
            FlattenNodePathSegment::List,
        ];
        let mut collected = vec![];
        super::traverse_and_callback_mut(
            &mut data,
            &path,
            &SchemaMetadata::default(),
            Some(GraphQLErrorPath::default()),
            &mut |_item, error_path| {
                collected.push(error_path.unwrap());
            },
        );
        assert_eq!(collected.len(), 2);
        assert_eq!(
            collected[0].segments,
            vec![
                GraphQLErrorPathSegment::String("items".into()),
                GraphQLErrorPathSegment::Index(0)
            ]
        );
        assert_eq!(
            collected[1].segments,
            vec![
                GraphQLErrorPathSegment::String("items".into()),
                GraphQLErrorPathSegment::Index(1)
            ]
        );
    }

    #[test]
    /**
     * Collect error paths for each item in a list at two levels
     * E.g. for data { users: [ { posts: [ {...}, {...} ] }, { posts: [ {...} ] } ] } and path ["users", List, "posts", List]
     * we should collect paths ["users", 0, "posts", 0], ["users", 0, "posts", 1], and ["users", 1, "posts", 0]
     */
    fn test_collect_error_paths_two_levels() {
        let mut data = Value::Object(vec![(
            "users",
            Value::Array(vec![
                Value::Object(vec![
                    ("id", Value::String("1".into())),
                    (
                        "posts",
                        Value::Array(vec![
                            Value::Object(vec![("id", Value::String("a".into()))]),
                            Value::Object(vec![("id", Value::String("b".into()))]),
                        ]),
                    ),
                ]),
                Value::Object(vec![
                    ("id", Value::String("2".into())),
                    (
                        "posts",
                        Value::Array(vec![Value::Object(vec![("id", Value::String("c".into()))])]),
                    ),
                ]),
            ]),
        )]);
        let path = vec![
            FlattenNodePathSegment::Field("users".into()),
            FlattenNodePathSegment::List,
            FlattenNodePathSegment::Field("posts".into()),
            FlattenNodePathSegment::List,
        ];
        let mut collected = vec![];
        super::traverse_and_callback_mut(
            &mut data,
            &path,
            &SchemaMetadata::default(),
            Some(GraphQLErrorPath::default()),
            &mut |_item, error_path| {
                collected.push(error_path.unwrap());
            },
        );
        assert_eq!(collected.len(), 3);
        assert_eq!(
            collected[0].segments,
            vec![
                GraphQLErrorPathSegment::String("users".into()),
                GraphQLErrorPathSegment::Index(0),
                GraphQLErrorPathSegment::String("posts".into()),
                GraphQLErrorPathSegment::Index(0),
            ]
        );
        assert_eq!(
            collected[1].segments,
            vec![
                GraphQLErrorPathSegment::String("users".into()),
                GraphQLErrorPathSegment::Index(0),
                GraphQLErrorPathSegment::String("posts".into()),
                GraphQLErrorPathSegment::Index(1),
            ]
        );
        assert_eq!(
            collected[2].segments,
            vec![
                GraphQLErrorPathSegment::String("users".into()),
                GraphQLErrorPathSegment::Index(1),
                GraphQLErrorPathSegment::String("posts".into()),
                GraphQLErrorPathSegment::Index(0),
            ]
        );
    }
}
