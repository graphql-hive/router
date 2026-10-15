use criterion::{criterion_group, criterion_main, Criterion};
use hive_router::query_planner::state::supergraph_state::SupergraphState;
use hive_router::query_planner::utils::parsing::parse_schema;
use std::hint::black_box;

fn supergraph_state_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("supergraph_state_building");

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
        // TODO: it's taking forever now
        ("bfg", "./fixture/bfg/supergraph.graphql"),
        ("spotify", "./fixture/spotify-supergraph.graphql"),
        (
            "provides_union",
            "./fixture/tests/provides-union-member.supergraph.graphql",
        ),
    ] {
        let supergraph_sdl =
            std::fs::read_to_string(schema_path).expect("Unable to read supergraph fixture");

        group.bench_function(name, |b| {
            b.iter(|| {
                let parsed_schema = parse_schema(black_box(&supergraph_sdl));
                black_box(SupergraphState::new(black_box(&parsed_schema)))
            })
        });
    }

    group.finish();
}

fn all_benchmarks(c: &mut Criterion) {
    supergraph_state_building(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
