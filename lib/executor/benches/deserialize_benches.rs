//! Subgraph response deserialization: JSON bytes -> `Value` tree.
//!
//! `*/raw` variants mark the leaf paths as custom-scalar paths, so they measure what the
//! existing `LazyValue`/`RawJson` path already buys. Generalizing that marking to every
//! eligible leaf is the point of the response-shape work, so this delta is the target.

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use hive_router_plan_executor::response::subgraph_response::SubgraphResponse;
use hive_router_query_planner::planner::response_shape::{ResponseShape, ResponseShapeField};
use std::hint::black_box;

pub mod payloads;

/// The same complete shape with nothing marked raw.
///
/// This — not "no shape at all" — is the baseline: the response tree is slot-addressed, so a
/// payload parsed without a shape lands in zero slots and is simply discarded. Every real
/// parse has a shape; the only question is whether anything in it is a passthrough.
fn strip_raw(shape: &ResponseShape) -> ResponseShape {
    let fields: Vec<ResponseShapeField> = shape
        .fields
        .iter()
        .map(|f| ResponseShapeField {
            key: f.key.clone(),
            shape: strip_raw(&f.shape),
        })
        .collect();
    let inert = fields.iter().all(|f| f.shape.inert);
    ResponseShape {
        fields,
        raw: false,
        inert,
    }
}

fn bench_payload(c: &mut Criterion, name: &str, json: String, shape: ResponseShape) {
    let mut group = c.benchmark_group("deserialize");
    group.throughput(Throughput::Bytes(json.len() as u64));
    let bytes = Bytes::from(json);

    let structured = strip_raw(&shape);
    for (variant, shape) in [("structured", &structured), ("raw", &shape)] {
        group.bench_function(format!("{name}/{variant}"), |b| {
            b.iter_batched(
                || bytes.clone(),
                |b| {
                    let resp = SubgraphResponse::deserialize_from_bytes(b, Some(shape)).unwrap();
                    black_box(resp);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn deserialize_benches(c: &mut Criterion) {
    use payloads::Shape::{Nested, Raw, Structured};

    bench_payload(
        c,
        "scalar_heavy_200",
        payloads::scalar_heavy(200),
        payloads::shape(&[(
            "users",
            Nested(&[
                ("__typename", Structured),
                ("id", Raw),
                ("name", Raw),
                ("username", Raw),
                ("email", Raw),
                ("bio", Raw),
            ]),
        )]),
    );

    bench_payload(
        c,
        "number_heavy_500",
        payloads::number_heavy(500),
        payloads::shape(&[(
            "metrics",
            Nested(&[
                ("__typename", Structured),
                ("id", Raw),
                ("count", Raw),
                ("ratio", Raw),
                ("total", Raw),
            ]),
        )]),
    );

    bench_payload(
        c,
        "scalar_lists_50x50",
        payloads::scalar_lists(50, 50),
        payloads::shape(&[(
            "products",
            Nested(&[
                ("__typename", Structured),
                ("id", Raw),
                ("tags", Raw),
                ("scores", Raw),
            ]),
        )]),
    );

    bench_payload(
        c,
        "mixed_200x10",
        payloads::mixed(200, 10),
        payloads::shape(&[(
            "articles",
            Nested(&[
                ("__typename", Structured),
                ("id", Structured),
                ("title", Structured),
                ("author", Structured),
                ("views", Structured),
                ("tags", Raw),
            ]),
        )]),
    );

    // Deeply nested objects, nothing to pass through: isolates per-object overhead.
    bench_payload(
        c,
        "deep_objects_d6_b3",
        payloads::deep_objects(6, 3),
        payloads::shape(&[(
            "root",
            Nested(&[
                ("__typename", Structured),
                ("id", Structured),
                ("children", Structured),
            ]),
        )]),
    );
}

criterion_group!(benches, deserialize_benches);
criterion_main!(benches);
