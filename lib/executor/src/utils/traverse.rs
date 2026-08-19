use std::collections::BTreeSet;

use hive_router_query_planner::planner::slot_path::SlotPathSegment;

use crate::{
    introspection::schema::{PossibleTypes, SchemaMetadata},
    response::{graphql_error::GraphQLErrorPath, value::Value},
};

fn entity_satisfies_any_type_condition(
    possible_types: &PossibleTypes,
    type_name: &str,
    type_conditions: &BTreeSet<String>,
) -> bool {
    type_conditions
        .iter()
        .any(|condition| possible_types.entity_satisfies_type_condition(type_name, condition))
}

/// A position with no `__typename` cannot be ruled out, so the gate passes — the same
/// outcome as the missing-key lookup this replaced.
#[inline]
fn passes_type_condition(
    value: &Value<'_>,
    typename_slot: Option<usize>,
    conditions: &BTreeSet<String>,
    possible_types: &PossibleTypes,
) -> bool {
    typename_slot
        .and_then(|slot| value.slot(slot))
        .and_then(Value::as_str)
        .is_none_or(|type_name| {
            entity_satisfies_any_type_condition(possible_types, type_name, conditions)
        })
}

pub fn traverse_and_callback_mut<'a, Callback>(
    current_data: &mut Value<'a>,
    remaining_path: &[SlotPathSegment],
    schema_metadata: &SchemaMetadata,
    current_error_path: Option<GraphQLErrorPath>,
    callback: &mut Callback,
) where
    Callback: FnMut(&mut Value<'a>, Option<GraphQLErrorPath>),
{
    let Some((segment, rest_of_path)) = remaining_path.split_first() else {
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
            callback(current_data, current_error_path);
        }
        return;
    };

    match segment {
        SlotPathSegment::List => {
            if let Value::Array(arr) = current_data {
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
        SlotPathSegment::Slot { slot, key } => {
            if let Some(next_data) = current_data.slot_mut(*slot) {
                let current_error_path_for_field = current_error_path
                    .map(|current_error_path| current_error_path.concat_str(key.clone()));
                traverse_and_callback_mut(
                    next_data,
                    rest_of_path,
                    schema_metadata,
                    current_error_path_for_field,
                    callback,
                );
            }
        }
        SlotPathSegment::TypenameEquals {
            typename_slot,
            conditions,
        } => {
            if current_data.is_object() {
                if passes_type_condition(
                    current_data,
                    *typename_slot,
                    conditions,
                    &schema_metadata.possible_types,
                ) {
                    traverse_and_callback_mut(
                        current_data,
                        rest_of_path,
                        schema_metadata,
                        current_error_path,
                        callback,
                    );
                }
            } else if let Value::Array(arr) = current_data {
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
    remaining_path: &'a [SlotPathSegment],
    possible_types: &'a PossibleTypes,
    callback: &mut Callback,
) where
    Callback: FnMut(&'a Value<'a>),
{
    let Some((segment, rest_of_path)) = remaining_path.split_first() else {
        if let Value::Array(arr) = current_data {
            for item in arr.iter() {
                callback(item);
            }
        } else {
            callback(current_data);
        }
        return;
    };

    match segment {
        SlotPathSegment::List => {
            if let Value::Array(arr) = current_data {
                for item in arr.iter() {
                    traverse_and_callback(item, rest_of_path, possible_types, callback);
                }
            }
        }
        SlotPathSegment::Slot { slot, .. } => {
            if let Some(next_data) = current_data.slot(*slot) {
                traverse_and_callback(next_data, rest_of_path, possible_types, callback);
            }
        }
        SlotPathSegment::TypenameEquals {
            typename_slot,
            conditions,
        } => {
            if current_data.is_object() {
                if passes_type_condition(current_data, *typename_slot, conditions, possible_types) {
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
    use hive_router_query_planner::planner::slot_path::SlotPathSegment;

    use crate::{
        introspection::schema::SchemaMetadata,
        response::{
            graphql_error::{GraphQLErrorPath, GraphQLErrorPathSegment},
            value::Value,
        },
    };

    fn slot(slot: usize, key: &str) -> SlotPathSegment {
        SlotPathSegment::Slot {
            slot,
            key: key.to_string(),
        }
    }

    /// `{ id: "<id>" }` at slot 0.
    fn entity(id: &str) -> Value<'static> {
        Value::Object(vec![Value::OwnedString(id.to_string().into())].into_boxed_slice())
    }

    #[test]
    /**
     * Collect error paths for each item in a list at one level
     * E.g. for data { items: [ {...}, {...} ] } and path ["items", List]
     * we should collect paths ["items", 0] and ["items", 1]
     */
    fn test_collect_error_paths_one_level() {
        let mut data = Value::Object(
            vec![Value::Array(vec![entity("1"), entity("2")].into_boxed_slice())].into_boxed_slice(),
        );
        let path = vec![slot(0, "items"), SlotPathSegment::List];
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
     * E.g. for data { users: [ { posts: [ {...}, {...} ] }, { posts: [ {...} ] } ] } and
     * path ["users", List, "posts", List] we should collect ["users", 0, "posts", 0],
     * ["users", 0, "posts", 1] and ["users", 1, "posts", 0]
     */
    fn test_collect_error_paths_two_levels() {
        // Each user is { id: <slot 0>, posts: <slot 1> }.
        let user = |id: &str, posts: Vec<Value<'static>>| {
            Value::Object(
                vec![
                    Value::OwnedString(id.to_string().into()),
                    Value::Array(posts.into_boxed_slice()),
                ]
                .into_boxed_slice(),
            )
        };
        let mut data = Value::Object(
            vec![Value::Array(
                vec![
                    user("1", vec![entity("a"), entity("b")]),
                    user("2", vec![entity("c")]),
                ]
                .into_boxed_slice(),
            )]
            .into_boxed_slice(),
        );

        let path = vec![
            slot(0, "users"),
            SlotPathSegment::List,
            slot(1, "posts"),
            SlotPathSegment::List,
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

        let expected = [(0usize, 0usize), (0, 1), (1, 0)];
        assert_eq!(collected.len(), expected.len());
        for (collected, (user_index, post_index)) in collected.iter().zip(expected) {
            assert_eq!(
                collected.segments,
                vec![
                    GraphQLErrorPathSegment::String("users".into()),
                    GraphQLErrorPathSegment::Index(user_index),
                    GraphQLErrorPathSegment::String("posts".into()),
                    GraphQLErrorPathSegment::Index(post_index),
                ]
            );
        }
    }
}
