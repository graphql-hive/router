use criterion::{BatchSize, BenchmarkId, Criterion};
use graphql_tools::parser::query;
use hive_router::executor::execution::entity_reuse::ReuseIndex;
use hive_router::executor::introspection::schema::{PossibleTypes, SchemaMetadata};
use hive_router::executor::response::merge::EntityMerger;
use hive_router::executor::response::value::Value;
use hive_router::executor::utils::traverse::traverse_and_callback_mut;
use hive_router::query_planner::ast::requires::RequiresSelectionSet;
use hive_router::query_planner::ast::selection_set::SelectionSet;
use hive_router::query_planner::planner::plan_nodes::{EntityReuse, FlattenNodePath, PathSegment};
use hive_router::query_planner::utils::parsing::parse_operation;
use std::hint::black_box;

const TARGETS: usize = 1000;

fn entity(i: usize) -> Value<'static> {
    let review = |n: usize| {
        Value::Object(vec![
            ("id", Value::String(format!("r{i}-{n}").into())),
            (
                "body",
                Value::String("Pretty good, would buy again.".into()),
            ),
            ("rating", Value::U64(4)),
        ])
    };
    Value::Object(vec![
        ("__typename", Value::String("Product".into())),
        ("name", Value::String(format!("Product {i}").into())),
        ("price", Value::U64(100 + i as u64)),
        ("inStock", Value::Bool(true)),
        (
            "reviews",
            Value::Array(vec![review(0), review(1), review(2)]),
        ),
    ])
}

/// Merges `_entities` results into `TARGETS` list items, the way a flatten fetch does.
/// `unique` entities are spread over the targets, so `unique < TARGETS` means duplicates.
pub fn benchmarks(c: &mut Criterion) {
    let path = [PathSegment::Field("products".into()), PathSegment::List];
    let schema_metadata = SchemaMetadata::default();
    let mut group = c.benchmark_group("entity_merge");

    for (label, unique) in [
        ("unique", TARGETS),
        ("dup_50", TARGETS / 2),
        ("dup_90", TARGETS / 10),
    ] {
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
        let entity_indexes: Vec<Option<usize>> = (0..TARGETS).map(|i| Some(i % unique)).collect();

        group.bench_function(BenchmarkId::from_parameter(label), |b| {
            b.iter_batched(
                || (data.clone(), entities.clone()),
                |(mut data, mut entities)| {
                    let mut merger =
                        EntityMerger::new(&mut entities, entity_indexes.iter().flatten().copied());
                    let mut index = 0;
                    traverse_and_callback_mut(
                        &mut data,
                        &path,
                        &schema_metadata,
                        None,
                        &mut |target, _| {
                            let entity_index = entity_indexes.get(index);
                            index += 1;
                            if let Some(Some(entity_index)) = entity_index {
                                merger.merge_into(target, *entity_index);
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
    reuse_benchmarks(c);
}

fn selection(source: &str) -> RequiresSelectionSet {
    let document = parse_operation(source);
    let set = document
        .definitions
        .iter()
        .find_map(|definition| match definition {
            query::Definition::Operation(query::OperationDefinition::SelectionSet(set)) => {
                Some(set.clone())
            }
            _ => None,
        })
        .expect("a bare selection set");
    RequiresSelectionSet::from(&SelectionSet::from(set))
}

/// A fetch whose entities are all in the response already: finding them (`build`) and copying
/// them for every target (`copy`), the two steps that replace the subgraph call.
fn reuse_benchmarks(c: &mut Criterion) {
    let possible_types = PossibleTypes::default();
    let requires = selection("{ ... on Product { __typename upc } }");
    let reuse = EntityReuse {
        sources: vec![FlattenNodePath::from(vec![
            PathSegment::Field("products".into()),
            PathSegment::List,
        ])],
        copy: selection("{ ... on Product { __typename inStock shippingEstimate } }"),
    };
    let mut group = c.benchmark_group("entity_reuse");

    for count in [5, 100, 1000] {
        let data = Value::Object(vec![(
            "products",
            Value::Array(
                (0..count)
                    .map(|i| {
                        Value::Object(vec![
                            ("__typename", Value::String("Product".into())),
                            ("upc", Value::String(format!("{i}").into())),
                            ("name", Value::String(format!("Product {i}").into())),
                            ("price", Value::U64(100 + i as u64)),
                            ("weight", Value::U64(10)),
                            ("inStock", Value::Bool(true)),
                            ("shippingEstimate", Value::U64(5)),
                        ])
                    })
                    .collect(),
            ),
        )]);
        let hashes: Vec<u64> = match &data {
            Value::Object(fields) => match &fields[0].1 {
                Value::Array(items) => items
                    .iter()
                    .map(|item| item.to_hash(requires.root_selections(), &possible_types))
                    .collect(),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };

        group.bench_function(BenchmarkId::new("build", count), |b| {
            b.iter(|| {
                black_box(ReuseIndex::build(
                    &reuse,
                    &data,
                    requires.root_selections(),
                    &possible_types,
                ))
            });
        });
        let index = ReuseIndex::build(&reuse, &data, requires.root_selections(), &possible_types);
        group.bench_function(BenchmarkId::new("copy", count), |b| {
            b.iter(|| {
                for hash in &hashes {
                    black_box(index.copy_for(*hash).expect("every entity is there"));
                }
            });
        });
    }

    group.finish();
}
