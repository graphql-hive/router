//! Parses the subgraph responses of one bench/operation.graphql request, without the subgraphs.
//!
//! benches/deser/*.json are the bodies bench/subgraphs sent the router for that operation (entity
//! reuse on, so these 4 fetches are all of them), and *.graphql the queries it sent. To
//! re-record, put a logging proxy between the router and the subgraphs and send the operation
//! once.
//!
//! - `sonic` is how the router reads a body when it doesn't know the fields.
//! - `shaped` is how it reads one with the shape of the fetch's query, from a real plan.
//! - `sonic_dom` is sonic building its own tree from the same bytes, the speed a parser that
//!   knows its job can get to.

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use hive_router::executor::response::shaped::ResponseShape;
use hive_router::executor::response::subgraph_response::SubgraphResponse;
use hive_router::query_planner::ast::normalization::normalize_operation;
use hive_router::query_planner::ast::operation::SubgraphFetchOperation;
use hive_router::query_planner::graph::PlannerOverrideContext;
use hive_router::query_planner::planner::plan_nodes::PlanNode;
use hive_router::query_planner::planner::{Planner, QueryPlannerOptions};
use hive_router::query_planner::utils::cancellation::CancellationToken;
use hive_router::query_planner::utils::parsing::{parse_operation, parse_schema};
use std::hint::black_box;

use hive_router::{configure_global_allocator, RouterGlobalAllocator};
// the allocator the router runs with, since most of parsing is allocating
configure_global_allocator!();

const RESPONSES: [(&str, &str, &[u8]); 4] = [
    (
        "products",
        include_str!("deser/products.graphql"),
        include_bytes!("deser/products.json"),
    ),
    (
        "accounts",
        include_str!("deser/accounts.graphql"),
        include_bytes!("deser/accounts.json"),
    ),
    (
        "inventory",
        include_str!("deser/inventory.graphql"),
        include_bytes!("deser/inventory.json"),
    ),
    (
        "reviews",
        include_str!("deser/reviews.graphql"),
        include_bytes!("deser/reviews.json"),
    ),
];

type Parse = fn(&'static [u8], &ResponseShape);

const PARSERS: [(&str, Parse); 3] = [
    ("sonic", |body, _| {
        let response = SubgraphResponse::deserialize_from_bytes(Bytes::from_static(body), None);
        black_box(response.expect("a valid response"));
    }),
    ("shaped", |body, shape| {
        let response =
            SubgraphResponse::deserialize_with_shape(Bytes::from_static(body), shape, None);
        black_box(response.expect("a valid response"));
    }),
    ("sonic_dom", |body, _| {
        black_box(sonic_rs::from_slice::<sonic_rs::Value>(body).expect("valid JSON"));
    }),
];

fn fetches<'p>(node: &'p PlanNode, out: &mut Vec<&'p SubgraphFetchOperation>) {
    match node {
        PlanNode::Fetch(fetch) => out.push(&fetch.operation),
        PlanNode::BatchFetch(batch) => out.push(&batch.operation),
        PlanNode::Flatten(flatten) => fetches(&flatten.node, out),
        PlanNode::Sequence(sequence) => sequence.nodes.iter().for_each(|n| fetches(n, out)),
        PlanNode::Parallel(parallel) => parallel.nodes.iter().for_each(|n| fetches(n, out)),
        _ => unreachable!("the bench operation has no conditions, subscriptions or defers"),
    }
}

/// The shape of every recorded fetch, from the plan the router makes for the operation.
fn shapes() -> Vec<ResponseShape> {
    let supergraph = parse_schema(include_str!("../../../bench/supergraph.graphql"));
    let planner = Planner::new_from_supergraph(&supergraph, QueryPlannerOptions::default())
        .expect("a planner");
    let operation = parse_operation(include_str!("../../../bench/operation.graphql"));
    let normalized = normalize_operation(&planner.supergraph, &operation, None).expect("valid");
    let plan = planner
        .plan_from_normalized_operation(
            normalized.executable_operation(),
            PlannerOverrideContext::default(),
            &CancellationToken::new(),
        )
        .expect("a plan")
        .into_executable();
    let mut operations = Vec::new();
    fetches(plan.node.as_ref().expect("fetches"), &mut operations);

    RESPONSES
        .iter()
        .map(|(name, query, _)| {
            // the plan has to send what was recorded, or the fixtures are stale
            let operation = operations
                .iter()
                .find(|operation| operation.document_str.trim_end() == query.trim_end())
                .unwrap_or_else(|| panic!("the plan sends what {name}.graphql has"));
            let shape = operation.response_shape.as_deref().expect("a shape");
            shape.clone()
        })
        .collect()
}

fn benchmarks(c: &mut Criterion) {
    let shapes = shapes();
    // both have to read the exact same response
    for ((name, _, body), shape) in RESPONSES.iter().zip(&shapes) {
        let sonic = SubgraphResponse::deserialize_from_bytes(Bytes::from_static(body), None);
        let shaped =
            SubgraphResponse::deserialize_with_shape(Bytes::from_static(body), shape, None);
        assert_eq!(format!("{shaped:?}"), format!("{sonic:?}"), "{name}");
    }

    let mut group = c.benchmark_group("deser");
    for ((name, _, body), shape) in RESPONSES.iter().zip(&shapes) {
        group.throughput(Throughput::Bytes(body.len() as u64));
        for (parser, parse) in PARSERS {
            group.bench_function(BenchmarkId::new(parser, name), |b| {
                b.iter(|| parse(body, shape))
            });
        }
    }

    // Everything one request parses.
    let total = RESPONSES.iter().map(|(_, _, body)| body.len() as u64).sum();
    group.throughput(Throughput::Bytes(total));
    for (parser, parse) in PARSERS {
        group.bench_function(BenchmarkId::new(parser, "request"), |b| {
            b.iter(|| {
                for ((_, _, body), shape) in RESPONSES.iter().zip(&shapes) {
                    parse(body, shape)
                }
            })
        });
    }
    group.finish();
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
