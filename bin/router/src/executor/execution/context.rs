use std::{
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, OnceLock,
    },
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
        ast::{
            operation::{ContextAliasTemplate, SubgraphFetchOperation},
            requires::RequiresSelectionSet,
        },
        planner::plan_nodes::{FetchContext, FetchNodePathSegment, FetchRewrite, ResponsePathRef},
        state::entity_keys::KeysByType,
    },
};

pub(super) type RepresentationHashes = Arc<Vec<Option<u64>>>;

pub(super) struct ContextGroup<'a> {
    pub variables: Vec<(&'a str, Vec<u8>)>,
    pub representations: Vec<u8>,
    pub hash_to_index: AHashMap<u64, usize>,
}

/// Borrows existing values, including whole objects and arrays. Only projected
/// arrays need their own structure; their selected elements remain borrowed.
#[derive(serde::Serialize)]
#[serde(untagged)]
enum SelectedValue<'a, 'v> {
    Borrowed(&'a Value<'v>),
    Array(Vec<SelectedValue<'a, 'v>>),
    Null,
}

fn select_value<'a, 'v>(
    value: &'a Value<'v>,
    path: &[FetchNodePathSegment],
    possible_types: &PossibleTypes,
) -> Option<SelectedValue<'a, 'v>> {
    if path.is_empty() || value.is_null() {
        return Some(SelectedValue::Borrowed(value));
    }
    if let Value::Array(items) = value {
        return Some(SelectedValue::Array(
            items
                .iter()
                .map(|item| select_value(item, path, possible_types).unwrap_or(SelectedValue::Null))
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

struct ContextSource<'a> {
    parent_depth: usize,
    path: &'a [FetchNodePathSegment],
}

/// Groups entities by their context values while traversing one borrowed response.
/// The `'d` lifetime ties the cached ancestors to that response, so the cache
/// cannot outlive it or be reused for another one.
struct ContextGroups<'a, 'd, 'v> {
    names: Vec<&'a str>,
    sources: Vec<Vec<ContextSource<'a>>>,
    groups: Vec<ContextGroup<'a>>,
    group_by_values: AHashMap<Vec<Vec<u8>>, usize>,
    ancestor_depths: Vec<usize>,
    // ancestors the previous entity read from, one per depth in `ancestor_depths`.
    // `None` means the ancestor was missing
    last_ancestors: Vec<Option<&'d Value<'v>>>,
    last_group: Option<usize>,
}

impl<'a, 'd, 'v> ContextGroups<'a, 'd, 'v> {
    fn new(context: &'a Option<Box<FetchContext>>) -> Self {
        let names = FetchContext::group_variables(context);
        let rewrites = FetchContext::rewrites(context).unwrap_or_default();
        let sources: Vec<Vec<ContextSource<'a>>> = names
            .iter()
            .map(|name| {
                rewrites
                    .iter()
                    .filter(|rewrite| rewrite.rename_key_to == *name)
                    .map(|rewrite| {
                        let parent_depth = rewrite
                            .path
                            .iter()
                            .take_while(|s| matches!(s, FetchNodePathSegment::Parent))
                            .count();
                        ContextSource {
                            parent_depth,
                            path: &rewrite.path[parent_depth..],
                        }
                    })
                    .collect()
            })
            .collect();
        let mut ancestor_depths: Vec<usize> = sources
            .iter()
            .flatten()
            .map(|source| source.parent_depth)
            .collect();
        ancestor_depths.sort_unstable();
        ancestor_depths.dedup();

        let mut groups = Vec::new();
        if names.is_empty() {
            groups.push(ContextGroup {
                variables: Vec::new(),
                representations: vec![b'['],
                hash_to_index: AHashMap::new(),
            });
        }
        Self {
            names,
            sources,
            groups,
            group_by_values: AHashMap::new(),
            last_ancestors: vec![None; ancestor_depths.len()],
            ancestor_depths,
            last_group: None,
        }
    }

    fn resolve(
        &mut self,
        entity: &'d Value<'v>,
        objects: &[&'d Value<'v>],
        possible_types: &PossibleTypes,
    ) -> usize {
        if self.names.is_empty() {
            return 0;
        }
        let ancestor = |depth: usize| {
            if depth == 0 {
                Some(entity)
            } else {
                objects.get(objects.len().checked_sub(depth)?).copied()
            }
        };

        // Check each distinct depth once, updating the cache for this entity.
        // Include every candidate source, even those skipped by a type condition.
        // compared by address: the response is borrowed for 'd and can't change, so the same
        // address means the same object and thus the same selected values.
        let mut same_ancestors = true;
        for (depth, previous) in self.ancestor_depths.iter().zip(&mut self.last_ancestors) {
            let current = ancestor(*depth);
            same_ancestors &= match (*previous, current) {
                (Some(previous), Some(current)) => std::ptr::eq(previous, current),
                (None, None) => true,
                _ => false,
            };
            *previous = current;
        }
        if let Some(index) = self.last_group.filter(|_| same_ancestors) {
            return index;
        }

        let values: Vec<Vec<u8>> = self
            .sources
            .iter()
            .map(|sources| {
                let value = sources.iter().find_map(|source| {
                    select_value(ancestor(source.parent_depth)?, source.path, possible_types)
                });
                // Send null like Apollo does; the subgraph enforces argument nullability.
                sonic_rs::to_vec(&value.unwrap_or(SelectedValue::Null))
                    .expect("context value serializes")
            })
            .collect();
        let index = *self
            .group_by_values
            .entry(values)
            .or_insert_with_key(|values| {
                let index = self.groups.len();
                self.groups.push(ContextGroup {
                    variables: self
                        .names
                        .iter()
                        .copied()
                        .zip(values.iter().cloned())
                        .collect(),
                    representations: vec![b'['],
                    hash_to_index: AHashMap::new(),
                });
                index
            });
        self.last_group = Some(index);
        index
    }
}

/// Walks every flatten path, deduplicates the entities by hash and projects their
/// representations. Entities are grouped by the tuple of their context values (one request per
/// group), without context variables there is at most one group. Identical paths share hashes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn collect_context_entities<'a, 'd, 'v>(
    data: &'d Value<'v>,
    paths: impl IntoIterator<Item = ResponsePathRef<'a>>,
    requires: &RequiresSelectionSet,
    input_rewrites: Option<&[FetchRewrite]>,
    context: &'a Option<Box<FetchContext>>,
    possible_types: &PossibleTypes,
    subgraph_keys: Option<&KeysByType>,
) -> (Vec<ContextGroup<'a>>, Vec<RepresentationHashes>) {
    let mut contexts = ContextGroups::new(context);
    let mut path_hashes: Vec<RepresentationHashes> = Vec::new();
    let mut seen_paths: Vec<(ResponsePathRef<'a>, RepresentationHashes)> = Vec::new();
    let required_selections = requires.root_selections();
    for path in paths {
        if let Some((_, hashes)) = seen_paths.iter().find(|(seen, _)| *seen == path) {
            path_hashes.push(Arc::clone(hashes));
            continue;
        }
        let mut hashes = Vec::new();
        // the callback gets references into `data`, which the ancestor cache relies on
        let mut collect = |entity: &'d Value<'v>, objects: &[&'d Value<'v>]| {
            if entity.is_null() {
                hashes.push(None);
                return;
            }
            let group_index = contexts.resolve(entity, objects, possible_types);
            let entity_hash = entity.to_hash(required_selections, possible_types);
            // every distinct set of values has its own group, so the group index stands in for
            // the values. the same entity in two groups still gets two hashes.
            let hash = if contexts.names.is_empty() {
                entity_hash
            } else {
                let mut hasher = Xxh3::new();
                entity_hash.hash(&mut hasher);
                group_index.hash(&mut hasher);
                hasher.finish()
            };
            hashes.push(Some(hash));
            let group = &mut contexts.groups[group_index];
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
    for group in &mut contexts.groups {
        group.representations.push(b']');
    }
    (contexts.groups, path_hashes)
}

/// One request that carries several groups of contextual argument values, written out from a
/// [`ContextAliasTemplate`].
struct AliasedRequest {
    operation: SubgraphFetchOperation,
    /// The names of the per-group variables in each group
    variable_names: Vec<String>,
}

/// Keeps the [`AliasedRequest`]s of one execution around until it's done, since the subgraph
/// request and the jobs borrow from them. They're written per request, so they're dropped with
/// it instead of piling up in the plan cache.
///
/// Append-only, so nothing it hands out ever moves. Bucket `b` holds `2^b` requests and is
/// allocated when the first one lands in it, so a push goes straight to its slot.
#[derive(Default)]
pub struct AliasedRequests {
    len: AtomicUsize,
    buckets: [OnceLock<Box<[OnceLock<AliasedRequest>]>>; 32],
}

impl AliasedRequests {
    /// Writes out a request for every group in `groups` (the variable values of each group, in
    /// group order) and keeps it until the execution is done. Returns its operation and its
    /// variables, renamed for their group.
    pub(super) fn write(
        &self,
        template: &ContextAliasTemplate,
        groups: Vec<Vec<(&str, Vec<u8>)>>,
    ) -> (&SubgraphFetchOperation, Vec<(&str, Vec<u8>)>) {
        let count = groups.len();
        let variable_count = groups.iter().map(Vec::len).sum();
        let mut variable_names = Vec::with_capacity(variable_count);
        let mut values = Vec::with_capacity(variable_count);
        for (group, variables) in groups.into_iter().enumerate() {
            for (name, value) in variables {
                variable_names.push(
                    template
                        .variable(name, group)
                        .unwrap_or_else(|| name.to_string()),
                );
                values.push(value);
            }
        }
        let request = self.insert(AliasedRequest {
            operation: template.operation(count),
            variable_names,
        });
        let variables = request
            .variable_names
            .iter()
            .map(String::as_str)
            .zip(values)
            .collect();
        (&request.operation, variables)
    }

    fn insert(&self, request: AliasedRequest) -> &AliasedRequest {
        // 1-based, so request `i` sits in bucket `ilog2(i)`
        let index = self.len.fetch_add(1, Ordering::Relaxed) + 1;
        let bucket = index.ilog2() as usize;
        let slots = self.buckets[bucket]
            .get_or_init(|| (0..1usize << bucket).map(|_| OnceLock::new()).collect());
        // every index is handed out once, so the slot is always empty
        slots[index - (1 << bucket)].get_or_init(|| request)
    }
}

/// Turns the response of an aliased request for `groups` groups back into the response of the
/// original operation, so it can be merged like any other. The entities of every group are
/// concatenated under the original response key and error paths are pointed to them.
///
/// `count(key, group)` is the number of representations sent for the `key`-th response key of
/// `template` in `group`. A group's entities are padded or cut to that, so the indexes of the
/// following groups stay stable.
pub(super) fn unalias_response<'exec>(
    response: &mut SubgraphResponse<'exec>,
    template: &'exec ContextAliasTemplate,
    groups: usize,
    count: impl Fn(usize, usize) -> usize,
) {
    let keys = &template.response_keys;
    // where every group starts, plus where the last one ends
    let offsets: Vec<Vec<usize>> = (0..keys.len())
        .map(|key| {
            let mut offsets = Vec::with_capacity(groups + 1);
            offsets.push(0);
            for group in 0..groups {
                offsets.push(offsets[group] + count(key, group));
            }
            offsets
        })
        .collect();

    if let Value::Object(fields) = &mut response.data {
        // every group has its own range in the merged list, so the aliases can come back in any
        // order. a missing group or entity stays null, extra entities are dropped.
        let mut merged: Vec<Vec<Value<'exec>>> = offsets
            .iter()
            .map(|offsets| vec![Value::Null; offsets[groups]])
            .collect();
        for (alias, value) in std::mem::take(fields) {
            if let (Some((key, group)), Value::Array(items)) = (template.response_key(alias), value)
            {
                if group < groups {
                    let range = offsets[key][group]..offsets[key][group + 1];
                    for (slot, item) in merged[key][range].iter_mut().zip(items) {
                        *slot = item;
                    }
                }
            }
        }
        *fields = keys
            .iter()
            .map(String::as_str)
            .zip(merged.into_iter().map(Value::Array))
            .collect();
    }

    for error in response.errors.iter_mut().flatten() {
        let Some(path) = error.path.as_mut() else {
            continue;
        };
        let Some(GraphQLErrorPathSegment::String(first)) = path.segments.first() else {
            continue;
        };
        let Some((key, group)) = template
            .response_key(first)
            .filter(|(_, group)| *group < groups)
        else {
            continue;
        };
        let offsets = &offsets[key];
        if let Some(GraphQLErrorPathSegment::Index(index)) = path.segments.get_mut(1) {
            if *index >= offsets[group + 1] - offsets[group] {
                error.path = None;
                continue;
            }
            *index += offsets[group];
        }
        path.segments[0] = GraphQLErrorPathSegment::String(keys[key].clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_planner::{
        ast::selection_set::SelectionSet,
        planner::plan_nodes::{FlattenNodePath, KeyRenamer, PathSegment},
        utils::parsing::parse_operation,
    };

    #[test]
    fn contexts_group_and_deduplicate_by_ancestor_values() {
        let data: Value<'_> = sonic_rs::from_str(r#"{
            "users": [
                {"__typename":"User", "currency":"USD", "txs":[{"__typename":"Transaction","id":"1"}, {"__typename":"Transaction","id":"1"}]},
                {"__typename":"User", "currency":"EUR", "txs":[{"__typename":"Transaction","id":"1"}]},
                {"__typename":"User", "currency":null, "txs":[{"__typename":"Transaction","id":"1"}, null]},
                {"__typename":"User", "currency":"USD", "txs":[{"__typename":"Transaction","id":"1"}]}
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
        let rewrites = [
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
        let context = FetchContext::new(vec!["__context_0".into()], Some(rewrites.to_vec()));
        let (groups, hashes) = collect_context_entities(
            &data,
            [path.as_ref()],
            &requires,
            None,
            &context,
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
        // another parent with the same value lands in the same group
        assert_eq!(hashes[0][5], hashes[0][0]);
        assert!(groups[0].hash_to_index.contains_key(&hashes[0][0].unwrap()));
        assert!(!groups[0].hash_to_index.contains_key(&hashes[0][2].unwrap()));
    }

    #[test]
    fn ancestor_cache_invalidates_on_entity_local_values() {
        // all posts share the same user, only the entity-local code differs, so the cached
        // parent alone must not decide the group
        let data: Value<'_> = sonic_rs::from_str(
            r#"{"users":[{"currency":"USD","posts":[
                {"__typename":"Post","id":"1","code":"A"},
                {"__typename":"Post","id":"2","code":"B"},
                {"__typename":"Post","id":"3","code":"A"}
            ]}]}"#,
        )
        .unwrap();
        let path: FlattenNodePath = vec![
            PathSegment::Field("users".into()),
            PathSegment::List,
            PathSegment::Field("posts".into()),
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
        let rewrites = [
            KeyRenamer {
                path: vec![
                    FetchNodePathSegment::Parent,
                    FetchNodePathSegment::Key("currency".into()),
                ],
                rename_key_to: "__context_0".into(),
            },
            KeyRenamer {
                path: vec![FetchNodePathSegment::Key("code".into())],
                rename_key_to: "__context_1".into(),
            },
        ];
        let context = FetchContext::new(
            vec!["__context_0".into(), "__context_1".into()],
            Some(rewrites.to_vec()),
        );
        let possible_types = PossibleTypes::default();
        let (groups, hashes) = collect_context_entities(
            &data,
            [path.as_ref()],
            &requires,
            None,
            &context,
            &possible_types,
            None,
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].variables[1].1, b"\"A\"");
        assert_eq!(groups[1].variables[1].1, b"\"B\"");
        let hashes: Vec<u64> = hashes[0].iter().map(|hash| hash.unwrap()).collect();
        assert!(groups[0].hash_to_index.contains_key(&hashes[0]));
        assert!(groups[1].hash_to_index.contains_key(&hashes[1]));
        assert!(groups[0].hash_to_index.contains_key(&hashes[2]));

        // resolving the same sources again reuses the cached group, a different entity under
        // the same parent doesn't
        let Value::Object(root) = &data else {
            panic!("object");
        };
        let Some(Value::Array(users)) = Value::object_get(root, "users") else {
            panic!("users");
        };
        let user = &users[0];
        let Value::Object(user_fields) = user else {
            panic!("user");
        };
        let Some(Value::Array(posts)) = Value::object_get(user_fields, "posts") else {
            panic!("posts");
        };
        let objects = [&data, user];
        let mut contexts = ContextGroups::new(&context);
        let resolved: Vec<usize> = [0, 0, 1, 1, 2, 0]
            .into_iter()
            .map(|post| contexts.resolve(&posts[post], &objects, &possible_types))
            .collect();
        assert_eq!(resolved, vec![0, 0, 1, 1, 0, 0]);
    }

    #[test]
    fn ancestor_cache_tracks_every_parent_depth_across_paths() {
        // the currency is two parents up and the name one parent up. posts under the same user
        // reuse the cached group, a new user or org must not. the drafts path starts right
        // after the posts path ends, under different ancestors than its last entity
        let data: Value<'_> = sonic_rs::from_str(
            r#"{"orgs":[
                {"currency":"USD","users":[
                    {"name":"a",
                     "posts":[{"__typename":"Post","id":"1"},{"__typename":"Post","id":"2"}],
                     "drafts":[{"__typename":"Post","id":"5"}]},
                    {"name":"b","posts":[{"__typename":"Post","id":"3"}]}
                ]},
                {"currency":"EUR","users":[
                    {"name":"a",
                     "posts":[{"__typename":"Post","id":"4"}],
                     "drafts":[{"__typename":"Post","id":"6"}]}
                ]}
            ]}"#,
        )
        .unwrap();
        let path = |field: &str| -> FlattenNodePath {
            vec![
                PathSegment::Field("orgs".into()),
                PathSegment::List,
                PathSegment::Field("users".into()),
                PathSegment::List,
                PathSegment::Field(field.into()),
                PathSegment::List,
            ]
            .into()
        };
        let posts = path("posts");
        let drafts = path("drafts");
        let definition = parse_operation("{ __typename id }").definitions.remove(0);
        let graphql_tools::parser::query::Definition::Operation(
            graphql_tools::parser::query::OperationDefinition::SelectionSet(selection),
        ) = definition
        else {
            panic!("selection");
        };
        let requires = RequiresSelectionSet::from(&SelectionSet::from(selection));
        let rewrites = [
            KeyRenamer {
                path: vec![
                    FetchNodePathSegment::Parent,
                    FetchNodePathSegment::Parent,
                    FetchNodePathSegment::Key("currency".into()),
                ],
                rename_key_to: "__context_0".into(),
            },
            KeyRenamer {
                path: vec![
                    FetchNodePathSegment::Parent,
                    FetchNodePathSegment::Key("name".into()),
                ],
                rename_key_to: "__context_1".into(),
            },
        ];
        let context = FetchContext::new(
            vec!["__context_0".into(), "__context_1".into()],
            Some(rewrites.to_vec()),
        );
        let (groups, hashes) = collect_context_entities(
            &data,
            [posts.as_ref(), drafts.as_ref()],
            &requires,
            None,
            &context,
            &PossibleTypes::default(),
            None,
        );
        assert_eq!(
            groups
                .iter()
                .map(|group| (
                    std::str::from_utf8(&group.variables[0].1).unwrap(),
                    std::str::from_utf8(&group.variables[1].1).unwrap(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (r#""USD""#, r#""a""#),
                (r#""USD""#, r#""b""#),
                (r#""EUR""#, r#""a""#),
            ]
        );
        // posts 1 to 4, then drafts 5 and 6, with the group each one must land in
        let entities = hashes[0].iter().chain(hashes[1].iter());
        for (hash, group) in entities.zip([0, 0, 1, 2, 0, 2]) {
            assert!(groups[group].hash_to_index.contains_key(&hash.unwrap()));
        }
        assert_eq!(
            groups
                .iter()
                .map(|group| group.hash_to_index.len())
                .collect::<Vec<_>>(),
            vec![3, 1, 2]
        );
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
        let context = FetchContext::new(vec!["__context_0".into()], Some(Vec::new()));
        let (groups, _) = collect_context_entities(
            &data,
            [path.as_ref()],
            &requires,
            None,
            &context,
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
    fn excessive_parent_navigation_sends_null() {
        // u has only the root as an ancestor, but the context rewrite asks for three parents.
        // walking past the root must send null without panicking or dropping u's representation
        let data = sonic_rs::from_str(r#"{"u":{"__typename":"U","id":"1"}}"#).unwrap();
        let path: FlattenNodePath = vec![PathSegment::Field("u".into())].into();
        let definition = parse_operation("{ __typename id }").definitions.remove(0);
        let graphql_tools::parser::query::Definition::Operation(
            graphql_tools::parser::query::OperationDefinition::SelectionSet(selection),
        ) = definition
        else {
            panic!("selection");
        };
        let requires = RequiresSelectionSet::from(&SelectionSet::from(selection));
        let rewrites = [KeyRenamer {
            path: vec![FetchNodePathSegment::Parent; 3],
            rename_key_to: "__context_0".into(),
        }];
        let context = FetchContext::new(vec!["__context_0".into()], Some(rewrites.to_vec()));
        let (groups, hashes) = collect_context_entities(
            &data,
            [path.as_ref()],
            &requires,
            None,
            &context,
            &PossibleTypes::default(),
            None,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].variables, vec![("__context_0", b"null".to_vec())]);
        assert_eq!(
            groups[0].representations,
            br#"[{"__typename":"U","id":"1"}]"#
        );
        assert_eq!(hashes[0].len(), 1);
        assert!(hashes[0][0].is_some());
    }

    #[test]
    fn selection_borrows_whole_values() {
        let data: Value<'_> =
            sonic_rs::from_str(r#"{"payload":{"items":[{"code":"USD"},{"code":"EUR"}]}}"#).unwrap();
        let possible_types = PossibleTypes::default();
        let path = [FetchNodePathSegment::Key("payload".into())];
        let selected = select_value(&data, &path, &possible_types).unwrap();
        let SelectedValue::Borrowed(value) = &selected else {
            panic!("whole objects should remain borrowed");
        };
        let Value::Object(fields) = &data else {
            panic!("object");
        };
        assert!(std::ptr::eq(
            *value,
            Value::object_get(fields, "payload").unwrap()
        ));
        assert_eq!(
            sonic_rs::to_string(&selected).unwrap(),
            r#"{"items":[{"code":"USD"},{"code":"EUR"}]}"#
        );
    }

    #[test]
    fn projected_arrays_preserve_nested_shape_and_missing_values() {
        let data: Value<'_> =
            sonic_rs::from_str(r#"[[{"code":"USD"},{}],null,[{"code":"EUR"}]]"#).unwrap();
        let path = [FetchNodePathSegment::Key("code".into())];
        let selected = select_value(&data, &path, &PossibleTypes::default()).unwrap();
        assert_eq!(
            sonic_rs::to_string(&selected).unwrap(),
            r#"[["USD",null],null,["EUR"]]"#
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
    fn aliased_requests_keep_every_pushed_request() {
        use crate::query_planner::ast::{document::Document, operation::PlanningFetchOperation};

        let document = parse_operation(
            "query($representations: [_Any!]!) { _entities(representations: $representations) { __typename } }",
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
        operation.set_max_context_groups(usize::MAX, vec!["representations".to_string()], None);
        let template = operation.context_alias_template().unwrap();

        // enough to fill a few buckets, each with its own group count
        let store = AliasedRequests::default();
        let requests: Vec<_> = (0..100)
            .map(|i| {
                let groups = (0..i % 7 + 2)
                    .map(|_| vec![("representations", b"[]".to_vec())])
                    .collect();
                store.write(template, groups)
            })
            .collect();
        for (i, (operation, variables)) in requests.iter().enumerate() {
            let last = i % 7 + 1;
            assert_eq!(variables.len(), last + 1);
            assert_eq!(variables[last].0, format!("representations_{last}"));
            assert!(operation
                .document_str
                .contains(&format!("_entities_{last}:")));
        }
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
        operation.set_max_context_groups(15, vec!["representations".into(), "c".into()], None);
        let template = operation.context_alias_template().unwrap();

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
                GraphQLError {
                    message: "surplus entity".into(),
                    locations: None,
                    path: Some(GraphQLErrorPath {
                        segments: vec![
                            GraphQLErrorPathSegment::String("_entities_0".into()),
                            GraphQLErrorPathSegment::Index(2),
                            GraphQLErrorPathSegment::String("f".into()),
                        ],
                    }),
                    extensions: Default::default(),
                },
            ]),
            extensions: None,
            headers: None,
            bytes: None,
            status: None,
        };
        let counts = [2, 1, 1];
        unalias_response(&mut response, template, 3, |_, group| counts[group]);

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
        assert_eq!(errors[2].message, "surplus entity");
        assert!(errors[2].path.is_none());
    }
}
