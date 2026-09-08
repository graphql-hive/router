use crate::query_planner::planner::plan_nodes::{PathSegment, TypeCondition};

use crate::executor::{
    introspection::schema::{PossibleTypes, SchemaMetadata},
    response::{graphql_error::GraphQLErrorPath, value::Value},
    utils::consts::TYPENAME_FIELD_NAME,
};

fn entity_satisfies_any_type_condition(
    possible_types: &PossibleTypes,
    type_name: &str,
    condition: &TypeCondition,
) -> bool {
    condition
        .names()
        .iter()
        .any(|name| possible_types.entity_satisfies_type_condition(type_name, name))
}

pub fn traverse_and_callback_mut<'a, Callback>(
    current_data: &mut Value<'a>,
    remaining_path: &[PathSegment],
    schema_metadata: &SchemaMetadata,
    current_error_path: Option<GraphQLErrorPath>,
    callback: &mut Callback,
) where
    Callback: FnMut(&mut Value<'a>, Option<GraphQLErrorPath>),
{
    let mut error_path = current_error_path;
    walk_mut(
        current_data,
        remaining_path,
        schema_metadata,
        &mut error_path,
        callback,
    );
}

fn push_index(error_path: &mut Option<GraphQLErrorPath>, index: usize) {
    if let Some(error_path) = error_path.as_mut() {
        error_path.push_index(index);
    }
}

fn push_field(error_path: &mut Option<GraphQLErrorPath>, field: &str) {
    if let Some(error_path) = error_path.as_mut() {
        error_path.push_field(field);
    }
}

fn pop(error_path: &mut Option<GraphQLErrorPath>) {
    if let Some(error_path) = error_path.as_mut() {
        error_path.pop();
    }
}

fn walk_mut<'a, Callback>(
    current_data: &mut Value<'a>,
    remaining_path: &[PathSegment],
    schema_metadata: &SchemaMetadata,
    error_path: &mut Option<GraphQLErrorPath>,
    callback: &mut Callback,
) where
    Callback: FnMut(&mut Value<'a>, Option<GraphQLErrorPath>),
{
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            // If the path is empty, we call the callback on each item in the array
            // We iterate because we want the entity objects directly
            for (index, item) in arr.iter_mut().enumerate() {
                push_index(error_path, index);
                callback(item, error_path.clone());
                pop(error_path);
            }
        } else {
            // If the path is empty and current_data is not an array, just call the callback
            callback(current_data, error_path.clone());
        }
        return;
    }

    let Some((first, rest_of_path)) = remaining_path.split_first() else {
        return;
    };

    match first {
        PathSegment::List => {
            // If the key is List, we expect current_data to be an array
            if let Value::Array(arr) = current_data {
                for (index, item) in arr.iter_mut().enumerate() {
                    push_index(error_path, index);
                    walk_mut(item, rest_of_path, schema_metadata, error_path, callback);
                    pop(error_path);
                }
            }
        }
        PathSegment::Field(field_name) => {
            // If the key is Field, we expect current_data to be an object
            if let Value::Object(map) = current_data {
                let field_name: &str = field_name.as_ref();
                if let Ok(idx) = map.binary_search_by_key(&field_name, |(key, _)| key) {
                    let (_, next_data) = map.get_mut(idx).unwrap();
                    push_field(error_path, field_name);
                    walk_mut(
                        next_data,
                        rest_of_path,
                        schema_metadata,
                        error_path,
                        callback,
                    );
                    pop(error_path);
                }
            }
        }
        PathSegment::TypeCondition(type_condition) => {
            // If the key is Cast, we expect current_data to be an object or an array
            if let Value::Object(obj) = current_data {
                let maybe_type_name = obj
                    .binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| k)
                    .ok()
                    .and_then(|idx| obj[idx].1.as_str());

                if maybe_type_name.is_none_or(|type_name| {
                    entity_satisfies_any_type_condition(
                        &schema_metadata.possible_types,
                        type_name,
                        type_condition,
                    )
                }) {
                    // A type condition does not add a path step.
                    walk_mut(
                        current_data,
                        rest_of_path,
                        schema_metadata,
                        error_path,
                        callback,
                    );
                }
            } else if let Value::Array(arr) = current_data {
                // If the current data is an array, we need to check each item
                for (index, item) in arr.iter_mut().enumerate() {
                    push_index(error_path, index);
                    // Use `remaining_path`, not `rest_of_path`, so the type condition is checked
                    // again for each item.
                    walk_mut(item, remaining_path, schema_metadata, error_path, callback);
                    pop(error_path);
                }
            }
        }
    }
}

pub fn traverse_and_callback<'a, Callback>(
    current_data: &'a Value<'a>,
    remaining_path: &'a [PathSegment],
    possible_types: &'a PossibleTypes,
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

    let Some((first, rest_of_path)) = remaining_path.split_first() else {
        return;
    };

    match first {
        PathSegment::List => {
            if let Value::Array(arr) = current_data {
                for item in arr.iter() {
                    traverse_and_callback(item, rest_of_path, possible_types, callback);
                }
            }
        }
        PathSegment::Field(field_name) => {
            if let Value::Object(map) = current_data {
                let field_name: &str = field_name.as_ref();
                if let Ok(idx) = map.binary_search_by_key(&field_name, |(key, _)| key) {
                    let (_, next_data) = &map[idx];
                    traverse_and_callback(next_data, rest_of_path, possible_types, callback);
                }
            }
        }
        PathSegment::TypeCondition(type_condition) => {
            if let Value::Object(obj) = current_data {
                let maybe_type_name = obj
                    .binary_search_by_key(&TYPENAME_FIELD_NAME, |(k, _)| k)
                    .ok()
                    .and_then(|idx| obj[idx].1.as_str());

                if maybe_type_name.is_none_or(|type_name| {
                    entity_satisfies_any_type_condition(possible_types, type_name, type_condition)
                }) {
                    traverse_and_callback(current_data, rest_of_path, possible_types, callback);
                }
            } else if let Value::Array(arr) = current_data {
                for item in arr.iter() {
                    traverse_and_callback(item, remaining_path, possible_types, callback);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::query_planner::planner::plan_nodes::{FlattenNodePath, PathSegment, TypeCondition};

    fn path(steps: &[&str]) -> FlattenNodePath {
        steps
            .iter()
            .map(|step| match *step {
                "@" => PathSegment::List,
                s if s.starts_with('|') => PathSegment::TypeCondition(Box::new(
                    TypeCondition::from_names(s.trim_start_matches('|').split('|')),
                )),
                field => PathSegment::Field(field.into()),
            })
            .collect::<Vec<_>>()
            .into()
    }

    use crate::executor::{
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
        let path = path(&["items", "@"]);
        let mut collected = vec![];
        super::traverse_and_callback_mut(
            &mut data,
            path.as_slice(),
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
        let path = path(&["users", "@", "posts", "@"]);
        let mut collected = vec![];
        super::traverse_and_callback_mut(
            &mut data,
            path.as_slice(),
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

    #[test]
    fn error_paths_survive_type_conditions_and_sibling_branches() {
        let mut data = Value::Object(vec![(
            "media",
            Value::Array(vec![
                Value::Object(vec![
                    ("__typename", Value::String("Book".into())),
                    (
                        "pages",
                        Value::Array(vec![Value::Object(vec![("id", Value::String("a".into()))])]),
                    ),
                ]),
                Value::Object(vec![
                    ("__typename", Value::String("Book".into())),
                    (
                        "pages",
                        Value::Array(vec![Value::Object(vec![("id", Value::String("b".into()))])]),
                    ),
                ]),
            ]),
        )]);

        let path = path(&["media", "@", "|Book", "pages", "@"]);
        let mut collected = vec![];
        super::traverse_and_callback_mut(
            &mut data,
            path.as_slice(),
            &SchemaMetadata::default(),
            Some(GraphQLErrorPath::default()),
            &mut |_item, error_path| {
                collected.push(error_path.expect("error path is tracked").segments);
            },
        );

        use crate::executor::response::graphql_error::GraphQLErrorPathSegment::{Index, String};
        assert_eq!(
            collected,
            vec![
                vec![
                    String("media".into()),
                    Index(0),
                    String("pages".into()),
                    Index(0)
                ],
                vec![
                    String("media".into()),
                    Index(1),
                    String("pages".into()),
                    Index(0)
                ],
            ],
            "the second entity's path must not inherit steps from the first"
        );
    }

    #[test]
    fn traverse_matches_multi_type_cast() {
        let data = Value::Object(vec![("__typename", Value::String("Book".into()))]);
        let path = path(&["|Book|User"]);
        let mut matched = false;

        super::traverse_and_callback(&data, path.as_slice(), &Default::default(), &mut |_value| {
            matched = true;
        });

        assert!(matched);
    }

    #[test]
    fn traverse_rejects_non_matching_multi_type_cast() {
        let data = Value::Object(vec![("__typename", Value::String("Magazine".into()))]);
        let path = path(&["|Book|User"]);
        let mut matched = false;

        super::traverse_and_callback(&data, path.as_slice(), &Default::default(), &mut |_value| {
            matched = true;
        });

        assert!(!matched);
    }
}
