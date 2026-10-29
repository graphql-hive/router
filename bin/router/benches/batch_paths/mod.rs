//! The walks of a BatchFetch alias with many merge paths that share a prefix.
//!
//! The data is `products.@.reviews.@` with N entity fields on every review, and the alias has
//! N merge paths, `products.@.reviews.@.e{k}`. So the paths share everything but the last step.
//! The loops copy what `collect_path_values`, `collect_context_entities` and
//! `merge_batch_entities` do (all private), built from the same public pieces. The trie is built
//! on every run, like a request does.

use ahash::{HashMap as AHashMap, HashMapExt};
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use graphql_tools::parser::query::{Definition, OperationDefinition};
use hive_router::executor::{
    introspection::schema::SchemaMetadata,
    projection::request::project_requires,
    response::{merge::deep_merge_from, value::Value},
    utils::traverse::PathTrie,
};
use hive_router::query_planner::{
    ast::{requires::RequiresSelectionSet, selection_set::SelectionSet},
    planner::plan_nodes::{FlattenNodePath, PathSegment},
    utils::parsing::parse_operation,
};
use std::hint::black_box;

const PRODUCTS: usize = 100;
const REVIEWS: usize = 10;
/// Distinct users, so most representations repeat and get deduplicated
const USERS: usize = 500;

fn field(name: &str) -> PathSegment {
    PathSegment::Field(name.into())
}

fn user<'a>(id: usize) -> Value<'a> {
    Value::Object(vec![
        ("__typename", Value::String("User".into())),
        ("id", Value::String(format!("u{id}").into())),
    ])
}

fn data<'a>(keys: &'a [String]) -> Value<'a> {
    let mut next = 0;
    let products = (0..PRODUCTS)
        .map(|p| {
            let reviews = (0..REVIEWS)
                .map(|r| {
                    let mut review = vec![
                        ("__typename", Value::String("Review".into())),
                        ("id", Value::String(format!("r{p}-{r}").into())),
                    ];
                    for key in keys {
                        // a cheap spread, so ids repeat across paths and reviews
                        next = (next * 31 + 7) % USERS;
                        review.push((key.as_str(), user(next)));
                    }
                    Value::Object(review)
                })
                .collect();
            Value::Object(vec![
                ("__typename", Value::String("Product".into())),
                ("id", Value::String(format!("p{p}").into())),
                ("reviews", Value::Array(reviews)),
            ])
        })
        .collect();
    Value::Object(vec![("products", Value::Array(products))])
}

/// Every value at the end of every path, path by path
fn walk<'d, 'v>(
    data: &'d Value<'v>,
    paths: &[FlattenNodePath],
    metadata: &SchemaMetadata,
) -> Vec<Vec<&'d Value<'v>>> {
    let trie = PathTrie::new(paths.iter().map(FlattenNodePath::as_slice));
    let ends = trie.ends();
    let mut by_node = vec![Vec::new(); trie.len()];
    trie.walk(data, &metadata.possible_types, &mut |node, value, _| {
        by_node[node].push(value)
    });
    ends.iter()
        .map(|end| std::mem::take(&mut by_node[*end]))
        .collect()
}

/// One hash list per path, plus the deduplicated representations, like `collect_context_entities`
/// without context variables
#[allow(clippy::type_complexity)]
fn collect<'v>(
    data: &Value<'v>,
    paths: &[FlattenNodePath],
    requires: &RequiresSelectionSet,
    metadata: &SchemaMetadata,
) -> (Vec<Vec<Option<u64>>>, AHashMap<u64, usize>, Vec<u8>) {
    let types = &metadata.possible_types;
    let selections = requires.root_selections();
    let mut hash_to_index = AHashMap::new();
    let mut representations = vec![b'['];
    let path_hashes = walk(data, paths, metadata)
        .into_iter()
        .map(|values| {
            values
                .into_iter()
                .map(|entity| {
                    if entity.is_null() {
                        return None;
                    }
                    let hash = entity.to_hash(selections, types);
                    if !hash_to_index.contains_key(&hash)
                        && project_requires(
                            types,
                            selections,
                            entity,
                            &mut representations,
                            hash_to_index.is_empty(),
                            None,
                            false,
                            None,
                        )
                    {
                        hash_to_index.insert(hash, hash_to_index.len());
                    }
                    Some(hash)
                })
                .collect()
        })
        .collect();
    representations.push(b']');
    (path_hashes, hash_to_index, representations)
}

/// Calls `callback` with the path index and the value, for every value at the end of every path
fn walk_mut<'a>(
    data: &mut Value<'a>,
    paths: &[FlattenNodePath],
    metadata: &SchemaMetadata,
    callback: &mut impl FnMut(usize, &mut Value<'a>),
) {
    // the paths are all different, so every node has at most one
    let trie = PathTrie::new(paths.iter().map(FlattenNodePath::as_slice));
    let ends = trie.ends();
    let mut path_by_node = vec![usize::MAX; trie.len()];
    for (path, end) in ends.iter().enumerate() {
        path_by_node[*end] = path;
    }
    trie.walk_mut(
        data,
        &metadata.possible_types,
        None,
        &mut |node, value, _| callback(path_by_node[node], value),
    );
}

pub fn benchmarks(c: &mut Criterion) {
    let metadata = SchemaMetadata::default();
    let requires = {
        let document = parse_operation("{ id }");
        let Definition::Operation(OperationDefinition::SelectionSet(selections)) =
            document.definitions.into_iter().next().unwrap()
        else {
            unreachable!()
        };
        let selections: SelectionSet = selections.into();
        RequiresSelectionSet::from(&selections)
    };

    let mut group = c.benchmark_group("batch_paths");
    for path_count in [1, 2, 4, 8, 10, 20, 50] {
        let keys: Vec<String> = (0..path_count).map(|k| format!("e{k:02}")).collect();
        let data = data(&keys);
        let paths: Vec<FlattenNodePath> = keys
            .iter()
            .map(|key| {
                vec![
                    field("products"),
                    PathSegment::List,
                    field("reviews"),
                    PathSegment::List,
                    field(key),
                ]
                .into()
            })
            .collect();
        group.throughput(Throughput::Elements(
            (path_count * PRODUCTS * REVIEWS) as u64,
        ));

        group.bench_function(BenchmarkId::new("collect_walk", path_count), |b| {
            b.iter(|| black_box(walk(&data, &paths, &metadata)))
        });

        group.bench_function(BenchmarkId::new("collect", path_count), |b| {
            b.iter(|| black_box(collect(&data, &paths, &requires, &metadata)))
        });

        // what the subgraph sends back: one entity per representation
        let (path_hashes, hash_to_index, _) = collect(&data, &paths, &requires, &metadata);
        let entities: Vec<Value> = (0..hash_to_index.len())
            .map(|_| {
                Value::Object(vec![
                    ("__typename", Value::String("User".into())),
                    ("name", Value::String("Some Name".into())),
                ])
            })
            .collect();

        group.bench_function(BenchmarkId::new("write_back_walk", path_count), |b| {
            b.iter_batched_ref(
                || data.clone(),
                |target| {
                    let mut count = 0usize;
                    walk_mut(target, &paths, &metadata, &mut |path, value| {
                        count += black_box(value).is_null() as usize + path;
                    });
                    black_box(count)
                },
                BatchSize::LargeInput,
            )
        });

        // minus the move on an entity's last use
        group.bench_function(BenchmarkId::new("write_back", path_count), |b| {
            b.iter_batched_ref(
                || data.clone(),
                |target| {
                    let mut next = vec![0; paths.len()];
                    walk_mut(target, &paths, &metadata, &mut |path, value| {
                        let hash = path_hashes[path].get(next[path]).copied();
                        next[path] += 1;
                        let Some(Some(hash)) = hash else {
                            return;
                        };
                        if let Some(entity_index) = hash_to_index.get(&hash) {
                            deep_merge_from(value, &entities[*entity_index]);
                        }
                    });
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}
