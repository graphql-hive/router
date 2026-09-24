use criterion::{BenchmarkId, Criterion, Throughput};
use hive_router::executor::{
    introspection::schema::SchemaWithMetadata,
    projection::{plan::ProjectionPlan, request::project_requires, response::project_by_operation},
    response::value::Value,
};
use hive_router::query_planner::ast::normalization::normalize_operation;
use hive_router::query_planner::ast::requires::RequiresSelectionSet;
use hive_router::query_planner::planner::Planner;
use hive_router::query_planner::utils::parsing::{parse_operation, parse_schema};
use std::hint::black_box;

const LIST_LENGTH: usize = 64;
const NARROW_FIELDS: usize = 16;
const WIDE_FIELDS: usize = 32;
/// Used only to pre-size the output buffer.
const BYTES_PER_FIELD: usize = 16;

pub fn benchmarks(c: &mut Criterion) {
    requires_benchmarks(c);
    list_benchmarks(c);
    abstract_type_benchmarks(c);
}

/// Projects lists with runtime type guards and field conditions.
fn abstract_type_benchmarks(c: &mut Criterion) {
    let schema = parse_schema(
        r#"
        type Query { nodes: [Node] }
        interface Node { id: ID! }
        type User implements Node { id: ID!, name: String, status: Status }
        type Admin implements Node { id: ID!, name: String, level: Int }
        type Guest implements Node { id: ID!, nickname: String }
        enum Status { ACTIVE PENDING BLOCKED ARCHIVED }
        "#,
    );
    let planner = Planner::new_from_supergraph(&schema, Default::default())
        .expect("Failed to create planner from supergraph");
    let schema_metadata = planner.consumer_schema.schema_metadata();
    let document = parse_operation(
        r#"
        {
          nodes {
            __typename
            id
            ... on User { name status }
            ... on Admin { name level }
            ... on Guest { nickname }
          }
        }
        "#,
    );
    let normalized = normalize_operation(&planner.supergraph, &document, None)
        .expect("Failed to normalize operation");
    let (root_type_name, plan) =
        ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);

    let user = |row: usize| {
        Value::Object(vec![
            ("__typename", Value::String("User".into())),
            ("id", Value::U64(row as u64)),
            ("name", Value::String("ada".into())),
            ("status", Value::String("ACTIVE".into())),
        ])
    };
    let admin = |row: usize| {
        Value::Object(vec![
            ("__typename", Value::String("Admin".into())),
            ("id", Value::U64(row as u64)),
            ("level", Value::U64(3)),
            ("name", Value::String("grace".into())),
        ])
    };
    let guest = |row: usize| {
        Value::Object(vec![
            ("__typename", Value::String("Guest".into())),
            ("id", Value::U64(row as u64)),
            ("nickname", Value::String("anon".into())),
        ])
    };

    let mut group = c.benchmark_group("projection_abstract");
    group.throughput(Throughput::Elements(LIST_LENGTH as u64));
    for (case, rows) in [
        (
            "mixed",
            (0..LIST_LENGTH)
                .map(|row| match row % 3 {
                    0 => user(row),
                    1 => admin(row),
                    _ => guest(row),
                })
                .collect::<Vec<_>>(),
        ),
        ("uniform", (0..LIST_LENGTH).map(user).collect::<Vec<_>>()),
    ] {
        let data = Value::Object(vec![("nodes", Value::Array(rows))]);
        group.bench_function(case, |b| {
            b.iter(|| {
                black_box(
                    project_by_operation(
                        black_box(&data),
                        vec![],
                        &Default::default(),
                        root_type_name,
                        black_box(&plan),
                        &None,
                        LIST_LENGTH * 4 * BYTES_PER_FIELD,
                        &schema_metadata,
                    )
                    .unwrap(),
                )
            });
        });
    }
    group.finish();
}

/// Covers both sides of the per-list-position cache threshold.
fn list_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("projection_lists");
    for field_count in [NARROW_FIELDS, WIDE_FIELDS] {
        let keys: Vec<String> = (0..field_count).map(|i| format!("field_{i:02}")).collect();
        let schema = parse_schema(&format!(
            "type Query {{ items: [Item] }} type Item {{ {} }}",
            keys.iter()
                .map(|key| format!("{key}: Int"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        let planner = Planner::new_from_supergraph(&schema, Default::default())
            .expect("Failed to create planner from supergraph");
        let schema_metadata = planner.consumer_schema.schema_metadata();
        // Reverse the selection order to check response order.
        let document = parse_operation(&format!(
            "{{ items {{ {} }} }}",
            keys.iter().rev().cloned().collect::<Vec<_>>().join(" ")
        ));
        let normalized = normalize_operation(&planner.supergraph, &document, None)
            .expect("Failed to normalize operation");
        let (root_type_name, plan) =
            ProjectionPlan::from_operation(normalized.executable_operation(), &schema_metadata);

        // Keys are stored in schema order, the opposite of the selection order above.
        let objects = (0..LIST_LENGTH)
            .map(|row| {
                Value::Object(
                    keys.iter()
                        .enumerate()
                        .map(|(i, key)| (key.as_str(), Value::U64((row + i) as u64)))
                        .collect(),
                )
            })
            .collect();
        let data = Value::Object(vec![("items", Value::Array(objects))]);

        group.throughput(Throughput::Elements(LIST_LENGTH as u64));
        group.bench_function(format!("fields_{field_count}"), |b| {
            b.iter(|| {
                black_box(
                    project_by_operation(
                        black_box(&data),
                        vec![],
                        &Default::default(),
                        root_type_name,
                        black_box(&plan),
                        &None,
                        LIST_LENGTH * field_count * BYTES_PER_FIELD,
                        &schema_metadata,
                    )
                    .unwrap(),
                )
            });
        });
    }
    group.finish();
}

fn requires_benchmarks(c: &mut Criterion) {
    use graphql_tools::parser::query::{Definition, OperationDefinition};
    use hive_router::query_planner::{
        ast::selection_set::SelectionSet, utils::parsing::parse_operation,
    };

    let requires_from = |body: &str| {
        let document = parse_operation(&format!("{{ {body} }}"));
        let Definition::Operation(OperationDefinition::SelectionSet(selections)) =
            document.definitions.into_iter().next().unwrap()
        else {
            unreachable!()
        };
        let selections: SelectionSet = selections.into();
        RequiresSelectionSet::from(&selections)
    };

    let keys: Vec<_> = (0..50).map(|i| format!("field_{i:02}")).collect();
    let mut entries = vec![("__typename", Value::String("Item".into()))];
    entries.extend(keys.iter().map(|key| (key.as_str(), Value::U64(42))));
    let data = Value::Object(entries);
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
        let requires = requires_from(&fields);
        let input = format!("{fragments}_fragments");
        bench_execution(&mut group, &requires, &data, &input);
        bench_serialization(&mut group, &requires, &input);
    }

    const NESTED: &str = "a: id b: sku nested { c: inner deep { d: leaf } } \
                          ... on Product @skip(if: $s) @include(if: $i) { upc dimensions { size weight } }";
    let nested_requires = requires_from(NESTED);
    let nested_json: sonic_rs::Value = sonic_rs::from_str(
        r#"{"__typename":"Product","id":1,"sku":2,
            "nested":{"inner":3,"deep":{"leaf":4}},
            "upc":5,"dimensions":{"size":6,"weight":7}}"#,
    )
    .unwrap();
    let nested_data = Value::from(nested_json.as_ref());
    bench_execution(
        &mut group,
        &nested_requires,
        &nested_data,
        "nested_aliased_directives",
    );
    bench_serialization(&mut group, &nested_requires, "nested_aliased_directives");
    group.finish();
}

fn bench_execution(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    requires: &RequiresSelectionSet,
    data: &Value<'_>,
    input: &str,
) {
    let types = Default::default();
    group.bench_function(BenchmarkId::new("hash", input), |b| {
        b.iter(|| {
            black_box(black_box(data).to_hash(black_box(requires.root_selections()), &types))
        });
    });
    group.bench_function(BenchmarkId::new("project", input), |b| {
        let mut buffer = Vec::with_capacity(512);
        b.iter(|| {
            buffer.clear();
            project_requires(
                &types,
                black_box(requires.root_selections()),
                black_box(data),
                &mut buffer,
                true,
                None,
            );
            black_box(&buffer);
        });
    });
}

fn bench_serialization(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    requires: &RequiresSelectionSet,
    input: &str,
) {
    group.bench_function(BenchmarkId::new("serialize", input), |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(requires)).unwrap()));
    });
}
