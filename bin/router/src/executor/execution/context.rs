use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use ahash::{HashMap as AHashMap, HashMapExt};
use xxhash_rust::xxh3::Xxh3;

use crate::{
    executor::{
        execution::rewrites::FetchRewriteExt,
        introspection::schema::PossibleTypes,
        projection::request::project_requires,
        response::{
            graphql_error::GraphQLErrorPathSegment, subgraph_response::SubgraphResponse,
            value::Value,
        },
        utils::traverse::traverse_and_callback,
    },
    query_planner::{
        ast::{operation::ContextAliasedOperation, requires::RequiresSelectionSet},
        planner::plan_nodes::{FetchNodePathSegment, FetchRewrite, KeyRenamer, ResponsePathRef},
        state::entity_keys::KeysByType,
    },
};

pub(super) type RepresentationHashes = Arc<Vec<Option<u64>>>;

pub(super) struct ContextGroup<'a> {
    pub variables: Vec<(&'a str, Vec<u8>)>,
    pub representations: Vec<u8>,
    pub hash_to_index: AHashMap<u64, usize>,
}

fn select_value<'v>(
    value: &Value<'v>,
    path: &[FetchNodePathSegment],
    possible_types: &PossibleTypes,
) -> Option<Value<'v>> {
    if path.is_empty() || value.is_null() {
        return Some(value.clone());
    }
    if let Value::Array(items) = value {
        return Some(Value::Array(
            items
                .iter()
                .map(|item| select_value(item, path, possible_types).unwrap_or_default())
                .collect(),
        ));
    }
    let Value::Object(object) = value else {
        return None;
    };
    match &path[0] {
        FetchNodePathSegment::Key(key) => {
            select_value(Value::object_get(object, key)?, &path[1..], possible_types)
        }
        FetchNodePathSegment::TypenameEquals(names) => {
            let name = Value::object_get(object, "__typename")?.as_str()?;
            names
                .iter()
                .any(|condition| possible_types.entity_satisfies_type_condition(name, condition))
                .then(|| select_value(value, &path[1..], possible_types))
                .flatten()
        }
        FetchNodePathSegment::Parent => None,
    }
}

/// Walks every flatten path, deduplicates the entities by hash and projects their
/// representations. Entities are grouped by the tuple of their context values (one request per
/// group), without context variables there is at most one group. Identical paths share hashes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn collect_context_entities<'a>(
    data: &Value<'_>,
    paths: impl IntoIterator<Item = ResponsePathRef<'a>>,
    requires: &RequiresSelectionSet,
    input_rewrites: Option<&[FetchRewrite]>,
    context_rewrites: Option<&'a [KeyRenamer]>,
    context_variables: &'a [String],
    possible_types: &PossibleTypes,
    subgraph_keys: Option<&KeysByType>,
) -> (Vec<ContextGroup<'a>>, Vec<RepresentationHashes>) {
    let mut names: Vec<&str> = context_variables.iter().map(String::as_str).collect();
    let rewrites = context_rewrites.unwrap_or_default();
    for rewrite in rewrites {
        if !names.contains(&rewrite.rename_key_to.as_str()) {
            names.push(rewrite.rename_key_to.as_str());
        }
    }
    let mut groups: Vec<ContextGroup<'a>> = Vec::new();
    if names.is_empty() {
        groups.push(ContextGroup {
            variables: Vec::new(),
            representations: vec![b'['],
            hash_to_index: AHashMap::new(),
        });
    }
    let mut group_by_values: AHashMap<Vec<Vec<u8>>, usize> = AHashMap::new();
    let mut path_hashes: Vec<RepresentationHashes> = Vec::new();
    let mut seen_paths: Vec<(ResponsePathRef<'a>, RepresentationHashes)> = Vec::new();
    let required_selections = requires.root_selections();
    for path in paths {
        if let Some((_, hashes)) = seen_paths.iter().find(|(seen, _)| *seen == path) {
            path_hashes.push(Arc::clone(hashes));
            continue;
        }
        let mut hashes = Vec::new();
        let mut collect = |entity: &Value<'_>, objects: &[&Value<'_>]| {
            if entity.is_null() {
                hashes.push(None);
                return;
            }
            let values: Vec<Vec<u8>> = names
                .iter()
                .map(|name| {
                    let value = rewrites
                        .iter()
                        .filter(|rewrite| rewrite.rename_key_to == *name)
                        .find_map(|rewrite| {
                            let levels = rewrite
                                .path
                                .iter()
                                .take_while(|s| matches!(s, FetchNodePathSegment::Parent))
                                .count();
                            let ancestor = if levels == 0 {
                                entity
                            } else {
                                *objects.get(objects.len().checked_sub(levels)?)?
                            };
                            select_value(ancestor, &rewrite.path[levels..], possible_types)
                        });
                    // send null like apollo does and let the subgraph enforce argument nullability
                    sonic_rs::to_vec(&value.unwrap_or_default()).expect("context value serializes")
                })
                .collect();
            let group_index = if names.is_empty() {
                0
            } else if let Some(index) = group_by_values.get(&values) {
                *index
            } else {
                let index = groups.len();
                groups.push(ContextGroup {
                    variables: names.iter().copied().zip(values.iter().cloned()).collect(),
                    representations: vec![b'['],
                    hash_to_index: AHashMap::new(),
                });
                group_by_values.insert(values.clone(), index);
                index
            };
            let entity_hash = entity.to_hash(required_selections, possible_types);
            let hash = if values.is_empty() {
                entity_hash
            } else {
                let mut hasher = Xxh3::new();
                entity_hash.hash(&mut hasher);
                values.hash(&mut hasher);
                hasher.finish()
            };
            hashes.push(Some(hash));
            let group = &mut groups[group_index];
            if group.hash_to_index.contains_key(&hash) {
                return;
            }
            let mut rewritten;
            let projected = if let Some(rewrites) = input_rewrites {
                rewritten = entity.clone();
                for rewrite in rewrites {
                    rewrite.rewrite(possible_types, &mut rewritten);
                }
                &rewritten
            } else {
                entity
            };
            if project_requires(
                possible_types,
                required_selections,
                projected,
                &mut group.representations,
                group.hash_to_index.is_empty(),
                None,
                false,
                subgraph_keys,
            ) {
                group.hash_to_index.insert(hash, group.hash_to_index.len());
            }
        };
        traverse_and_callback(data, path.as_slice(), possible_types, &mut collect);
        let hashes = Arc::new(hashes);
        seen_paths.push((path, Arc::clone(&hashes)));
        path_hashes.push(hashes);
    }
    for group in &mut groups {
        group.representations.push(b']');
    }
    (groups, path_hashes)
}

/// Turns the response of a [`ContextAliasedOperation`] back into the response of the original
/// operation, so it can be merged like any other. The entities of every group are concatenated
/// under the original response key and error paths are pointed to them.
///
/// `count(key, group)` is the number of representations sent for `key` in `group`. A group's
/// entities are padded or cut to that, so the indexes of the following groups stay stable.
pub(super) fn unalias_response<'exec>(
    response: &mut SubgraphResponse<'exec>,
    aliased: &'exec ContextAliasedOperation,
    count: impl Fn(&str, usize) -> usize,
) {
    let offsets: Vec<Vec<usize>> = aliased
        .response_keys
        .iter()
        .map(|(key, aliases)| {
            let mut offset = 0;
            (0..aliases.len())
                .map(|group| {
                    let current = offset;
                    offset += count(key, group);
                    current
                })
                .collect()
        })
        .collect();

    if let Value::Object(fields) = &mut response.data {
        let mut merged = Vec::with_capacity(aliased.response_keys.len());
        for ((key, aliases), offsets) in aliased.response_keys.iter().zip(&offsets) {
            let mut entities = Vec::new();
            for (group, alias) in aliases.iter().enumerate() {
                let count = count(key, group);
                if let Some(position) = Value::object_position(fields, alias) {
                    if let Value::Array(items) = fields.swap_remove(position).1 {
                        entities.extend(items.into_iter().take(count));
                    }
                }
                entities.resize(offsets[group] + count, Value::Null);
            }
            merged.push((key.as_str(), Value::Array(entities)));
        }
        *fields = merged;
    }

    for error in response.errors.iter_mut().flatten() {
        let Some(path) = error.path.as_mut() else {
            continue;
        };
        let Some(GraphQLErrorPathSegment::String(first)) = path.segments.first() else {
            continue;
        };
        let Some((key, offset)) =
            aliased
                .response_keys
                .iter()
                .zip(&offsets)
                .find_map(|((key, aliases), offsets)| {
                    let group = aliases.iter().position(|alias| alias == first)?;
                    Some((key, offsets[group]))
                })
        else {
            continue;
        };
        path.segments[0] = GraphQLErrorPathSegment::String(key.clone());
        if let Some(GraphQLErrorPathSegment::Index(index)) = path.segments.get_mut(1) {
            *index += offset;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::{
        ast::selection_set::SelectionSet,
        planner::plan_nodes::{FlattenNodePath, PathSegment},
        utils::parsing::parse_operation,
    };

    #[test]
    fn contexts_group_and_deduplicate_by_ancestor_values() {
        let data: Value<'_> = sonic_rs::from_str(r#"{
            "users": [
                {"__typename":"User", "currency":"USD", "txs":[{"__typename":"Transaction","id":"1"}, {"__typename":"Transaction","id":"1"}]},
                {"__typename":"User", "currency":"EUR", "txs":[{"__typename":"Transaction","id":"1"}]},
                {"__typename":"User", "currency":null, "txs":[{"__typename":"Transaction","id":"1"}, null]}
            ]
        }"#).unwrap();
        let path: FlattenNodePath = vec![
            PathSegment::Field("users".into()),
            PathSegment::List,
            PathSegment::Field("txs".into()),
            PathSegment::List,
        ]
        .into();
        let definition = parse_operation("{ __typename id }").definitions.remove(0);
        let graphql_tools::parser::query::Definition::Operation(
            graphql_tools::parser::query::OperationDefinition::SelectionSet(selection),
        ) = definition
        else {
            panic!("selection");
        };
        let requires = RequiresSelectionSet::from(&SelectionSet::from(selection));
        let rewrites = vec![
            KeyRenamer {
                path: vec![
                    FetchNodePathSegment::Parent,
                    FetchNodePathSegment::typename_equals_from_type("Other".into()),
                    FetchNodePathSegment::Key("currency".into()),
                ],
                rename_key_to: "__context_0".into(),
            },
            KeyRenamer {
                path: vec![
                    FetchNodePathSegment::Parent,
                    FetchNodePathSegment::typename_equals_from_type("User".into()),
                    FetchNodePathSegment::Key("currency".into()),
                ],
                rename_key_to: "__context_0".into(),
            },
        ];
        let names = ["__context_0".into()];
        let (groups, hashes) = collect_context_entities(
            &data,
            [path.as_ref()],
            &requires,
            None,
            Some(&rewrites),
            &names,
            &PossibleTypes::default(),
            None,
        );
        assert_eq!(groups.len(), 3);
        assert_eq!(
            groups.iter().map(|g| &g.variables[0].1).collect::<Vec<_>>(),
            vec![
                &b"\"USD\"".to_vec(),
                &b"\"EUR\"".to_vec(),
                &b"null".to_vec()
            ]
        );
        assert!(groups.iter().all(|g| g.hash_to_index.len() == 1));
        assert_eq!(hashes[0][0], hashes[0][1]);
        assert_ne!(hashes[0][0], hashes[0][2]);
        assert_eq!(hashes[0][4], None);
        assert!(groups[0].hash_to_index.contains_key(&hashes[0][0].unwrap()));
        assert!(!groups[0].hash_to_index.contains_key(&hashes[0][2].unwrap()));
    }

    #[test]
    fn empty_context_selection_sends_null() {
        let data = sonic_rs::from_str(r#"{"__typename":"Transaction","id":"1"}"#).unwrap();
        let path = FlattenNodePath::default();
        let definition = parse_operation("{ __typename id missing }")
            .definitions
            .remove(0);
        let graphql_tools::parser::query::Definition::Operation(
            graphql_tools::parser::query::OperationDefinition::SelectionSet(selection),
        ) = definition
        else {
            panic!("selection");
        };
        let requires = RequiresSelectionSet::from(&SelectionSet::from(selection));
        let keys: KeysByType = [("Transaction".into(), vec![vec!["id".into()]])]
            .into_iter()
            .collect();
        let names = ["__context_0".into()];
        let (groups, _) = collect_context_entities(
            &data,
            [path.as_ref()],
            &requires,
            None,
            Some(&[]),
            &names,
            &PossibleTypes::default(),
            Some(&keys),
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].variables, vec![("__context_0", b"null".to_vec())]);
        assert_eq!(groups[0].hash_to_index.len(), 1);
        assert_eq!(
            std::str::from_utf8(&groups[0].representations).unwrap(),
            r#"[{"__typename":"Transaction","id":"1","missing":null}]"#
        );
    }

    #[test]
    fn selection_preserves_lists_and_checks_types() {
        let data: Value<'_> = sonic_rs::from_str(
            r#"{"__typename":"User", "codes":[{"code":"USD"}, null, {"code":"EUR"}]}"#,
        )
        .unwrap();
        let possible_types = PossibleTypes::default();
        let path = vec![
            FetchNodePathSegment::typename_equals_from_type("User".into()),
            FetchNodePathSegment::Key("codes".into()),
            FetchNodePathSegment::Key("code".into()),
        ];
        let value = select_value(&data, &path, &possible_types).unwrap();
        assert_eq!(
            sonic_rs::to_string(&value).unwrap(),
            r#"["USD",null,"EUR"]"#
        );
    }

    #[test]
    fn unalias_response_concatenates_groups_and_shifts_error_paths() {
        use crate::{
            executor::response::graphql_error::{GraphQLError, GraphQLErrorPath},
            query_planner::ast::{document::Document, operation::PlanningFetchOperation},
        };

        let document = parse_operation(
            "query($representations: [_Any!]!, $c: ID) { _entities(representations: $representations) { ... on T { f(a: $c) } } }",
        );
        let graphql_tools::parser::query::Definition::Operation(operation) =
            document.definitions.into_iter().next().unwrap()
        else {
            panic!("operation");
        };
        let mut operation = PlanningFetchOperation::from_anonymous_operation(Document {
            operation: operation.into(),
            fragments: Vec::new(),
        })
        .operation;
        operation.set_max_context_groups(15);
        let aliased = operation
            .context_aliased(3, &["representations", "c"])
            .unwrap();

        let mut response = SubgraphResponse {
            // group 0 returns too many entities, group 1 none
            data: sonic_rs::from_str(
                r#"{"_entities_2":[{"f":3}],"_entities_0":[{"f":1},{"f":2},{"f":9}],"_entities_1":null}"#,
            )
            .unwrap(),
            errors: Some(vec![
                GraphQLError {
                    message: "boom".into(),
                    locations: None,
                    path: Some(GraphQLErrorPath {
                        segments: vec![
                            GraphQLErrorPathSegment::String("_entities_2".into()),
                            GraphQLErrorPathSegment::Index(0),
                            GraphQLErrorPathSegment::String("f".into()),
                        ],
                    }),
                    extensions: Default::default(),
                },
                GraphQLError::from_message_and_extensions("other", Default::default()),
            ]),
            extensions: None,
            headers: None,
            bytes: None,
            status: None,
        };
        let counts = [2, 1, 1];
        unalias_response(&mut response, aliased, |_, group| counts[group]);

        assert_eq!(
            sonic_rs::to_string(&response.data).unwrap(),
            r#"{"_entities":[{"f":1},{"f":2},null,{"f":3}]}"#
        );
        let errors = response.errors.unwrap();
        assert_eq!(
            errors[0].path.as_ref().unwrap().segments,
            vec![
                GraphQLErrorPathSegment::String("_entities".into()),
                GraphQLErrorPathSegment::Index(3),
                GraphQLErrorPathSegment::String("f".into()),
            ]
        );
        assert!(errors[1].path.is_none());
    }
}
