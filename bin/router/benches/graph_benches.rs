use criterion::{criterion_group, criterion_main, Criterion};
use hive_router::query_planner::graph::Graph;
use hive_router::query_planner::state::supergraph_state::SupergraphState;
use hive_router::query_planner::utils::parsing::parse_schema;
use std::hint::black_box;

fn graph_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_building");

    for (name, schema_path) in [
        ("bench", "../../bench/supergraph.graphql"),
        (
            "many_plans",
            "./fixture/grafbase-many-plans/supergraph.graphql",
        ),
        (
            "abstract_many_subgraphs",
            "./fixture/abstract-many-subgraphs/supergraph.graphql",
        ),
        ("heavy_query", "./fixture/heavy-query/supergraph.graphql"),
        // TODO: ("bfg", "./fixture/bfg/supergraph.graphql"), as it's taking forever now
        ("spotify", "./fixture/spotify-supergraph.graphql"),
        (
            "provides_union",
            "./fixture/tests/provides-union-member.supergraph.graphql",
        ),
    ] {
        let supergraph_sdl =
            std::fs::read_to_string(schema_path).expect("Unable to read supergraph fixture");
        let parsed_schema = parse_schema(&supergraph_sdl);
        let supergraph_state = SupergraphState::new(&parsed_schema);

        group.bench_function(name, |b| {
            b.iter(|| {
                black_box(
                    Graph::graph_from_supergraph_state(black_box(&supergraph_state))
                        .expect("failed to create graph"),
                )
            })
        });
    }

    group.finish();
}

fn all_benchmarks(c: &mut Criterion) {
    graph_building(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
