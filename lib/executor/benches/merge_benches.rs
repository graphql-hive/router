//! `deep_merge` as a federated object accumulates fields across sequential fetches.
//!
//! Every fetch that writes into a position is deserialized against that position's shape, so
//! all the payloads here share one shape — which is what lets the merge be a positional walk
//! instead of a keyed rebuild.

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use hive_router_plan_executor::response::{
    merge::deep_merge, subgraph_response::SubgraphResponse, value::Value,
};
use hive_router_query_planner::planner::response_shape::{ResponseShape, ResponseShapeField};
use std::hint::black_box;

fn leaf() -> ResponseShape {
    ResponseShape {
        fields: Vec::new(),
        raw: false,
        inert: true,
    }
}

/// `{ users: [ <keys> ] }`, the merged shape every fetch into this position shares.
fn shape_for(keys: &[String]) -> ResponseShape {
    let user = ResponseShape {
        fields: keys
            .iter()
            .map(|key| ResponseShapeField {
                key: key.clone(),
                shape: leaf(),
            })
            .collect(),
        raw: false,
        inert: true,
    };
    ResponseShape {
        fields: vec![ResponseShapeField {
            key: "users".to_string(),
            shape: user,
        }],
        raw: false,
        inert: true,
    }
}

/// One fetch's worth of an entity list, writing only the keys it owns.
fn payload(rows: usize, keys: &[String]) -> String {
    let mut out = String::from(r#"{"users":["#);
    for row in 0..rows {
        if row > 0 {
            out.push(',');
        }
        out.push_str(r#"{"__typename":"User""#);
        for key in keys {
            out.push_str(&format!(r#","{key}":"value {key} {row}""#));
        }
        out.push('}');
    }
    out.push_str("]}");
    out
}

fn fetch_keys(fetch_idx: usize, fields: usize) -> Vec<String> {
    (0..fields)
        .map(|f| format!("f{fetch_idx}_{f}"))
        .collect()
}

/// Keys that sort *between* the ones already present, the worst case for a keyed merge.
fn interleaved_keys(fetch_idx: usize, fields: usize) -> Vec<String> {
    (0..fields)
        .map(|f| format!("field_{:03}", f * 10 + fetch_idx))
        .collect()
}

struct Fixture {
    responses: Vec<SubgraphResponse<'static>>,
}

impl Fixture {
    /// All fetches parse against the union of every key, exactly as the merged shape does.
    fn new(rows: usize, per_fetch_keys: Vec<Vec<String>>) -> Self {
        let mut all_keys = vec!["__typename".to_string()];
        for keys in &per_fetch_keys {
            all_keys.extend(keys.iter().cloned());
        }
        all_keys[1..].sort();
        let shape = shape_for(&all_keys);

        let responses = per_fetch_keys
            .iter()
            .map(|keys| {
                SubgraphResponse::deserialize_from_bytes(
                    Bytes::from(format!(r#"{{"data":{}}}"#, payload(rows, keys))),
                    Some(&shape),
                )
                .expect("payload")
            })
            .collect();

        Fixture { responses }
    }

    fn target_and_rest(&self) -> (Value<'static>, Vec<Value<'static>>) {
        (
            self.responses[0].data.clone(),
            self.responses[1..].iter().map(|r| r.data.clone()).collect(),
        )
    }
}

fn bench_case<M: criterion::measurement::Measurement>(
    group: &mut criterion::BenchmarkGroup<'_, M>,
    name: &str,
    fixture: &Fixture,
) {
    group.bench_function(name, |b| {
        b.iter_batched(
            || fixture.target_and_rest(),
            |(mut target, rest)| {
                for source in rest {
                    deep_merge(&mut target, source);
                }
                black_box(target);
            },
            BatchSize::SmallInput,
        );
    });
}

fn merge_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("deep_merge");

    for fetches in [2usize, 5, 10] {
        let fixture = Fixture::new(
            100,
            (0..fetches).map(|i| fetch_keys(i, 10)).collect(),
        );
        bench_case(
            &mut group,
            &format!("sequential_fetches/{fetches}x10_fields"),
            &fixture,
        );
    }

    // Keys that sort between the existing ones. With a positional merge this should cost the
    // same as the sequential case — key order stopped mattering.
    let interleaved = Fixture::new(100, (0..10).map(|i| interleaved_keys(i, 10)).collect());
    bench_case(&mut group, "interleaved_keys/10x10_fields", &interleaved);

    // Every fetch writes the same keys: a re-entry fetch, or one response fanned out.
    let overlapping = Fixture::new(100, (0..5).map(|_| fetch_keys(0, 10)).collect());
    bench_case(&mut group, "overlapping_fields/5x10_fields", &overlapping);

    // The realistic federation shape: an entity with ~20 fields total, each fetch supplying
    // a handful. A positional merge costs the width of the *shape*, not of the payload, so
    // this ratio is what decides whether it pays.
    let realistic = Fixture::new(100, (0..4).map(|i| fetch_keys(i, 5)).collect());
    bench_case(&mut group, "realistic/4_fetches_x5_of_20_fields", &realistic);

    // One merge, many keys on both sides.
    let wide = Fixture::new(1, vec![fetch_keys(0, 60), fetch_keys(1, 60)]);
    bench_case(&mut group, "wide_object/60_plus_60", &wide);

    group.finish();
}

criterion_group!(benches, merge_benches);
criterion_main!(benches);
