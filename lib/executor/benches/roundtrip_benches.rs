//! Deserialize + project, the pair that actually decides whether raw passthrough pays.
//!
//! Measuring deserialization alone is misleading: `RawJson` moves work from projection
//! (number formatting, string escaping) into parsing (`LazyValue`), so only the round trip
//! shows the net.

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use graphql_tools::parser::query::Definition;
use hive_router_plan_executor::{
    introspection::schema::{SchemaMetadata, SchemaWithMetadata},
    projection::{plan::FieldProjectionPlan, response::project_by_operation},
    response::subgraph_response::SubgraphResponse,
};
use hive_router_query_planner::{
    ast::{document::NormalizedDocument, normalization::create_normalized_document},
    consumer_schema::ConsumerSchema,
    planner::{merged_shape::response_shape_for_operation, response_shape::ResponseShape},
    state::supergraph_state::SupergraphState,
    utils::parsing::{parse_operation, parse_schema},
};
use std::hint::black_box;

pub mod payloads;

const SDL: &str = r#"
    type Query {
        users: [User!]!
        metrics: [Metric!]!
        products: [Product!]!
        articles: [Article!]!
    }
    type User { id: ID! name: String! username: String! email: String! bio: String! }
    type Metric { id: ID! count: Int! ratio: Float! total: Int! }
    # Nullable item lists: a non-null item would need its structure preserved for null
    # propagation, so the planner would not mark it as a passthrough.
    type Product { id: ID! tags: [String] scores: [Int] }
    type Article { id: ID! title: String! author: String! views: Int! tags: [String] }
"#;

struct Fixture {
    metadata: &'static SchemaMetadata,
    root_type_name: &'static str,
    plan: Vec<FieldProjectionPlan>,
    /// The shape the projection plan resolved its slots from — the payload has to be parsed
    /// against this one, or projection reads the wrong slots and silently emits nulls.
    shape: ResponseShape,
}

fn fixture(operation_source: &str) -> Fixture {
    let supergraph = parse_schema(SDL);
    let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
    // Leaked on purpose: the projection plan and root type name borrow from these, and a
    // bench fixture lives for the whole process anyway (same trick as `executor_benches`).
    let metadata: &'static SchemaMetadata = Box::leak(Box::new(consumer_schema.schema_metadata()));
    let mut operation = parse_operation(operation_source);
    let operation_ast = operation
        .definitions
        .iter_mut()
        .find_map(|def| match def {
            Definition::Operation(op) => Some(op),
            _ => None,
        })
        .expect("operation");
    let supergraph_state = SupergraphState::new(&supergraph);
    let normalized: &'static NormalizedDocument = Box::leak(Box::new(
        create_normalized_document(&supergraph_state, operation_ast.clone(), None),
    ));
    let (root_type_name, plan) =
        FieldProjectionPlan::from_operation(&normalized.operation, metadata);

    Fixture {
        metadata,
        root_type_name,
        plan,
        shape: response_shape_for_operation(&normalized.operation),
    }
}

/// Marks the given response paths as passthroughs on a copy of the fixture's shape.
fn with_raw_paths(shape: &ResponseShape, paths: &[&[&str]]) -> ResponseShape {
    let mut shape = shape.clone();
    for path in paths {
        shape.insert_raw_path(path.iter());
    }
    shape
}

fn bench_roundtrip(
    c: &mut Criterion,
    name: &str,
    operation: &str,
    json: String,
    raw_paths: &[&[&str]],
) {
    let fixture = fixture(operation);
    let mut group = c.benchmark_group("roundtrip");
    group.throughput(Throughput::Bytes(json.len() as u64));
    let json_len = json.len();
    let size_hint = json_len * 12 / 10;
    let bytes = Bytes::from(json);

    let raw_shape = with_raw_paths(&fixture.shape, raw_paths);
    for (variant, shape) in [("structured", &fixture.shape), ("raw", &raw_shape)] {
        // A bench that projects nothing would look like a speedup, so make that impossible.
        let response =
            SubgraphResponse::deserialize_from_bytes(bytes.clone(), Some(shape)).unwrap();
        let sample = project_by_operation(
            &response.data,
            vec![],
            &Default::default(),
            fixture.root_type_name,
            &fixture.plan,
            &None,
            size_hint,
            fixture.metadata,
        )
        .unwrap();
        assert!(
            sample.len() > json_len / 2,
            "{name}/{variant} projected only {} bytes from a {} byte payload — the shape and \
             the projection plan disagree on slots",
            sample.len(),
            json_len
        );

        group.bench_function(format!("{name}/{variant}"), |b| {
            b.iter_batched(
                || bytes.clone(),
                |b| {
                    let response =
                        SubgraphResponse::deserialize_from_bytes(b, Some(shape)).unwrap();
                    let out = project_by_operation(
                        &response.data,
                        vec![],
                        &Default::default(),
                        fixture.root_type_name,
                        &fixture.plan,
                        &None,
                        size_hint,
                        fixture.metadata,
                    )
                    .unwrap();
                    black_box(out);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn roundtrip_benches(c: &mut Criterion) {
    bench_roundtrip(
        c,
        "scalar_heavy_200",
        "{ users { id name username email bio } }",
        payloads::scalar_heavy(200),
        &[
            &["users", "id"],
            &["users", "name"],
            &["users", "username"],
            &["users", "email"],
            &["users", "bio"],
        ],
    );

    bench_roundtrip(
        c,
        "number_heavy_500",
        "{ metrics { id count ratio total } }",
        payloads::number_heavy(500),
        &[
            &["metrics", "id"],
            &["metrics", "count"],
            &["metrics", "ratio"],
            &["metrics", "total"],
        ],
    );

    bench_roundtrip(
        c,
        "scalar_lists_50x50",
        "{ products { id tags scores } }",
        payloads::scalar_lists(50, 50),
        &[&["products", "tags"], &["products", "scores"]],
    );

    bench_roundtrip(
        c,
        "mixed_200x10",
        "{ articles { id title author views tags } }",
        payloads::mixed(200, 10),
        &[&["articles", "tags"]],
    );
}

criterion_group!(benches, roundtrip_benches);
criterion_main!(benches);
