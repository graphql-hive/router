use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use ahash::{HashMap as AHashMap, HashMapExt};
use xxhash_rust::xxh3::Xxh3;

use crate::{
    executor::{
        execution::rewrites::FetchRewriteExt, introspection::schema::PossibleTypes,
        projection::request::project_requires, response::value::Value,
        utils::traverse::traverse_and_callback,
    },
    query_planner::{
        ast::requires::RequiresSelectionSet,
        planner::plan_nodes::{FetchNodePathSegment, FetchRewrite, KeyRenamer, ResponsePathRef},
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
/// group), without `context_rewrites` there is at most one group. Identical paths share hashes.
pub(super) fn collect_context_entities<'a>(
    data: &Value<'_>,
    paths: impl IntoIterator<Item = ResponsePathRef<'a>>,
    requires: &RequiresSelectionSet,
    input_rewrites: Option<&[FetchRewrite]>,
    context_rewrites: Option<&'a [KeyRenamer]>,
    possible_types: &PossibleTypes,
) -> (Vec<ContextGroup<'a>>, Vec<RepresentationHashes>) {
    let mut names = Vec::new();
    let rewrites = context_rewrites.unwrap_or_default();
    for rewrite in rewrites {
        if !names.contains(&rewrite.rename_key_to.as_str()) {
            names.push(rewrite.rename_key_to.as_str());
        }
    }
    let mut groups: Vec<ContextGroup<'a>> = Vec::new();
    let mut group_by_values = HashMap::new();
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
            let group_index = *group_by_values.entry(values.clone()).or_insert_with(|| {
                let index = groups.len();
                groups.push(ContextGroup {
                    variables: names.iter().copied().zip(values.iter().cloned()).collect(),
                    representations: vec![b'['],
                    hash_to_index: AHashMap::new(),
                });
                index
            });
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
            ) {
                group.hash_to_index.insert(hash, group.hash_to_index.len());
            }
        };
        traverse_and_callback(
            data,
            path.as_slice(),
            possible_types,
            &mut Vec::new(),
            &mut collect,
        );
        let hashes = Arc::new(hashes);
        seen_paths.push((path, Arc::clone(&hashes)));
        path_hashes.push(hashes);
    }
    for group in &mut groups {
        group.representations.push(b']');
    }
    (groups, path_hashes)
}

pub(super) fn batch_operation_without_empty_aliases(
    batch: &crate::query_planner::planner::plan_nodes::BatchFetchNode,
    prepared: &[(Vec<ContextGroup<'_>>, Vec<RepresentationHashes>)],
) -> Option<String> {
    use crate::query_planner::{
        ast::{
            fragment::FragmentDefinition, operation::OperationDefinition,
            selection_item::SelectionItem,
        },
        utils::parsing::parse_operation,
    };
    use graphql_tools::parser::query::Definition;
    let empty: Vec<_> = batch
        .entity_batch
        .aliases
        .iter()
        .zip(prepared)
        .filter(|(_, (groups, _))| groups.is_empty())
        .map(|(alias, _)| alias)
        .collect();
    if empty.is_empty() {
        return None;
    }
    let mut operation: Option<OperationDefinition> = None;
    let mut fragments: Vec<FragmentDefinition> = Vec::new();
    for definition in parse_operation(&batch.operation.document_str).definitions {
        match definition {
            Definition::Operation(op) => operation = Some(op.into()),
            Definition::Fragment(fragment) => fragments.push(fragment.into()),
        }
    }
    let mut operation = operation.expect("batch operation");
    operation.selection_set = operation
        .selection_set
        .inline_fragment_spreads(&fragments)
        .expect("batch fragments");
    operation.selection_set.items.retain(|item| {
        !matches!(item, SelectionItem::Field(field) if empty.iter().any(|alias| field.alias.as_deref() == Some(&alias.alias)))
    });
    let usages = operation.selection_set.variable_usages();
    if let Some(variables) = &mut operation.variable_definitions {
        variables.retain(|variable| usages.contains(&variable.name));
    }
    // an empty alias has no context value, so omit its non-null variables rather than invalidating the other aliases
    Some(operation.to_string())
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
        let (groups, hashes) = collect_context_entities(
            &data,
            [path.as_ref()],
            &requires,
            None,
            Some(&rewrites),
            &PossibleTypes::default(),
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
    fn empty_batch_aliases_drop_non_null_context_and_unused_client_variables() {
        use crate::query_planner::planner::plan_nodes::BatchFetchNode;
        let batch: BatchFetchNode = serde_json::from_value(serde_json::json!({
            "id": 0, "serviceName": "a", "operation": {
                "document_str": "query($r0:[_Any!]!,$r1:[_Any!]!,$c0:String!,$c1:String!,$locale:String){_e0:_entities(representations:$r0){... on Transaction{amount(code:$c0,locale:$locale)}} _e1:_entities(representations:$r1){... on Transaction{amount(code:$c1)}}}",
                "hash": 0, "name_write_position": 5,
            },
            "entityBatch": {"aliases": [
                {"alias":"_e0", "representationsVariableName":"r0", "paths":[], "requires":[], "contextRewrites":[{"path":[], "renameKeyTo":"c0"}]},
                {"alias":"_e1", "representationsVariableName":"r1", "paths":[], "requires":[], "contextRewrites":[{"path":[], "renameKeyTo":"c1"}]}
            ]}
        })).unwrap();
        let prepared = vec![
            (Vec::new(), Vec::new()),
            (
                vec![ContextGroup {
                    variables: vec![("c1", b"\"USD\"".to_vec())],
                    representations: b"[]".to_vec(),
                    hash_to_index: AHashMap::new(),
                }],
                Vec::new(),
            ),
        ];
        let operation = batch_operation_without_empty_aliases(&batch, &prepared).unwrap();
        assert!(!operation.contains("_e0"));
        assert!(!operation.contains("$c0"));
        assert!(!operation.contains("$locale"));
        assert!(operation.contains("$c1:String!"));
        assert!(operation.contains("_e1"));
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
}
