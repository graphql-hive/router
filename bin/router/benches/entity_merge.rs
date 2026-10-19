use ahash::{HashMap as AHashMap, HashMapExt};
use criterion::{BatchSize, BenchmarkId, Criterion};
use hive_router::executor::introspection::schema::SchemaMetadata;
use hive_router::executor::response::merge::{deep_merge, EntitiesToMerge};
use hive_router::executor::response::value::Value;
use hive_router::executor::utils::traverse::traverse_and_callback_mut;
use hive_router::query_planner::planner::plan_nodes::PathSegment;
use std::hint::black_box;

const TARGETS: usize = 1000;

fn entity(i: usize) -> Value<'static> {
    let review = |n: usize| {
        Value::Object(vec![
            ("id", Value::String(format!("r{i}-{n}").into())),
            ("body", Value::String("Pretty good, would buy again.".into())),
            ("rating", Value::U64(4)),
        ])
    };
    Value::Object(vec![
        ("__typename", Value::String("Product".into())),
        ("name", Value::String(format!("Product {i}").into())),
        ("price", Value::U64(100 + i as u64)),
        ("inStock", Value::Bool(true)),
        ("reviews", Value::Array(vec![review(0), review(1), review(2)])),
    ])
}

/// Merges `_entities` results into `TARGETS` list items, the way a flatten fetch does.
/// `unique` entities are spread over the targets, so `unique < TARGETS` means duplicates.
pub fn benchmarks(c: &mut Criterion) {
    let path = [PathSegment::Field("products".into()), PathSegment::List];
    let schema_metadata = SchemaMetadata::default();
    let mut group = c.benchmark_group("entity_merge");

    for (label, unique) in [("unique", TARGETS), ("dup_50", TARGETS / 2), ("dup_90", TARGETS / 10)]
    {
        let data = Value::Object(vec![(
            "products",
            Value::Array(
                (0..TARGETS)
                    .map(|i| {
                        Value::Object(vec![
                            ("__typename", Value::String("Product".into())),
                            ("upc", Value::String(format!("{}", i % unique).into())),
                        ])
                    })
                    .collect(),
            ),
        )]);
        let entities: Vec<Value> = (0..unique).map(entity).collect();
        let representation_hashes: Vec<Option<u64>> =
            (0..TARGETS).map(|i| Some((i % unique) as u64)).collect();
        let mut representation_hash_to_index = AHashMap::new();
        for i in 0..unique {
            representation_hash_to_index.insert(i as u64, i);
        }

        group.bench_function(BenchmarkId::from_parameter(label), |b| {
            b.iter_batched(
                || (data.clone(), entities.clone()),
                |(mut data, mut entities)| {
                    let mut entities = EntitiesToMerge::new(
                        &mut entities,
                        representation_hashes
                            .iter()
                            .flatten()
                            .filter_map(|hash| representation_hash_to_index.get(hash))
                            .copied(),
                    );
                    let mut index = 0;
                    traverse_and_callback_mut(
                        &mut data,
                        &path,
                        &schema_metadata,
                        None,
                        &mut |target, _| {
                            let hash = representation_hashes.get(index).copied();
                            index += 1;
                            let Some(Some(hash)) = hash else {
                                return;
                            };
                            if let Some(entity_index) = representation_hash_to_index.get(&hash) {
                                if let Some(entity) = entities.next_for(*entity_index) {
                                    deep_merge(target, entity);
                                }
                            }
                        },
                    );
                    black_box(data)
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}
