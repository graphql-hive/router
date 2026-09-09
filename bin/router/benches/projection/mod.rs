use criterion::{BenchmarkId, Criterion, Throughput};
use hive_router::executor::{
    introspection::schema::{FieldNullability, SchemaMetadata},
    projection::{
        plan::{FieldProjectionPlan, ProjectionValueSource},
        request::project_requires,
        response::project_by_operation,
    },
    response::value::Value,
};
use std::{hint::black_box, sync::Arc};

fn field(name: &str, selections: Option<Vec<FieldProjectionPlan>>) -> FieldProjectionPlan {
    FieldProjectionPlan {
        field_name: name.into(),
        response_key: name.into(),
        is_typename: name == "__typename",
        nullability: FieldNullability::Leaf { non_null: false },
        parent_type_guard: None,
        conditions: None,
        value: ProjectionValueSource::ResponseData {
            selections: selections.map(Arc::new),
        },
    }
}

pub fn benchmarks(c: &mut Criterion) {
    requires_benchmarks(c);
    // Build the Value and plan outside the timed loop, so this measures projection
    // (including its output allocation), rather than parsing or planning.
    let schema = SchemaMetadata::default();
    let mut group = c.benchmark_group("projection_lists");
    for field_count in [5, 15, 20, 50] {
        let keys: Vec<_> = (0..field_count).map(|i| format!("field_{i:02}")).collect();
        let selected_count = field_count.min(10);
        let plans = vec![field(
            "items",
            Some(
                // Reverse the selection order to ensure response order is preserved.
                (0..selected_count)
                    .rev()
                    .map(|i| field(&keys[i * field_count / selected_count], None))
                    .collect(),
            ),
        )];
        for count in [1, 2, 4, 8, 10, 100, 1_000, 10_000] {
            for mixed in [false, true] {
                let objects = (0..count)
                    .map(|row| {
                        Value::Object(
                            keys.iter()
                                .enumerate()
                                .filter(|(i, _)| !mixed || (row + i) % 7 != 0)
                                .map(|(i, key)| (key.as_str(), Value::U64(i as u64)))
                                .collect(),
                        )
                    })
                    .collect();
                let data = Value::Object(vec![("items", Value::Array(objects))]);
                let shape = if mixed { "mixed" } else { "uniform" };
                group.throughput(Throughput::Elements(count as u64));
                group.bench_with_input(
                    BenchmarkId::new(format!("{shape}_{field_count}_fields"), count),
                    &data,
                    |b, data| {
                        b.iter(|| {
                            black_box(
                                project_by_operation(
                                    black_box(data),
                                    vec![],
                                    &Default::default(),
                                    "Query",
                                    black_box(&plans),
                                    &None,
                                    count * selected_count * 16,
                                    &schema,
                                )
                                .unwrap(),
                            )
                        });
                    },
                );
            }
        }
    }
    // Wide shape: 32 selected fields is more than the stack cache holds,
    // so this exercises the heap path with few and many elements.
    // Small matrix on purpose.
    let wide_keys: Vec<_> = (0..32).map(|i| format!("wide_{i:02}")).collect();
    let wide_plans = vec![field(
        "items",
        Some((0..32).rev().map(|i| field(&wide_keys[i], None)).collect()),
    )];
    for count in [2, 8, 100] {
        for mixed in [false, true] {
            let objects = (0..count)
                .map(|row| {
                    Value::Object(
                        wide_keys
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| !mixed || (row + i) % 7 != 0)
                            .map(|(i, key)| (key.as_str(), Value::U64(i as u64)))
                            .collect(),
                    )
                })
                .collect();
            let data = Value::Object(vec![("items", Value::Array(objects))]);
            let shape = if mixed { "mixed" } else { "uniform" };
            group.throughput(Throughput::Elements(count as u64));
            group.bench_with_input(
                BenchmarkId::new(format!("{shape}_wide_32_fields"), count),
                &data,
                |b, data| {
                    b.iter(|| {
                        black_box(
                            project_by_operation(
                                black_box(data),
                                vec![],
                                &Default::default(),
                                "Query",
                                black_box(&wide_plans),
                                &None,
                                count * 32 * 16,
                                &schema,
                            )
                            .unwrap(),
                        )
                    });
                },
            );
        }
    }
    group.finish();

    // Each level creates deferred type context. With typename unselected, the
    // context stays unresolved; selecting it also exercises lazy resolution.
    let mut group = c.benchmark_group("projection_nested_lists");
    for with_typename in [false, true] {
        let mut plans = vec![field("value", None)];
        let mut data = Value::Object(vec![
            ("__typename", Value::String("Node".into())),
            ("value", Value::U64(42)),
        ]);
        for _ in 0..4 {
            if with_typename {
                plans.push(field("__typename", None));
            }
            plans = vec![field("children", Some(plans))];
            data = Value::Object(vec![
                ("__typename", Value::String("Node".into())),
                ("children", Value::Array(vec![data; 8])),
            ]);
        }
        group.bench_function(
            if with_typename {
                "resolved"
            } else {
                "deferred"
            },
            |b| {
                b.iter(|| {
                    black_box(
                        project_by_operation(
                            black_box(&data),
                            vec![],
                            &Default::default(),
                            "Query",
                            black_box(&plans),
                            &None,
                            200_000,
                            &schema,
                        )
                        .unwrap(),
                    )
                });
            },
        );
    }
    group.finish();
}

fn requires_benchmarks(c: &mut Criterion) {
    use graphql_tools::parser::query::{Definition, OperationDefinition};
    use hive_router::query_planner::{
        ast::selection_set::SelectionSet, utils::parsing::parse_operation,
    };

    let keys: Vec<_> = (0..50).map(|i| format!("field_{i:02}")).collect();
    let mut entries = vec![("__typename", Value::String("Item".into()))];
    entries.extend(keys.iter().map(|key| (key.as_str(), Value::U64(42))));
    let data = Value::Object(entries);
    let types = Default::default();
    let mut group = c.benchmark_group("requires_loops");
    for fragments in [0, 1, 8] {
        // Keep all eight fields selected; vary only how many are wrapped in fragments.
        let fields = (0..8)
            .map(|i| {
                if i < fragments {
                    format!("... on Item {{ field_{i:02} }}")
                } else {
                    format!("field_{i:02}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let document = parse_operation(&format!("{{ {fields} }}"));
        let Definition::Operation(OperationDefinition::SelectionSet(selections)) =
            document.definitions.into_iter().next().unwrap()
        else {
            unreachable!()
        };
        let selections: SelectionSet = selections.into();
        let input = format!("{fragments}_fragments");
        group.bench_function(BenchmarkId::new("hash_8_fields", &input), |b| {
            b.iter(|| black_box(black_box(&data).to_hash(black_box(&selections.items), &types)));
        });
        group.bench_function(BenchmarkId::new("project_8_fields", &input), |b| {
            let mut buffer = Vec::with_capacity(512);
            b.iter(|| {
                buffer.clear();
                project_requires(
                    &types,
                    black_box(&selections.items),
                    black_box(&data),
                    &mut buffer,
                    true,
                    None,
                );
                black_box(&buffer);
            });
        });
    }
    group.finish();
}
