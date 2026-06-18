use criterion::{criterion_group, criterion_main, Criterion};
use hive_router_query_planner::graph::Graph;
use hive_router_query_planner::state::supergraph_state::SupergraphState;
use hive_router_query_planner::utils::parsing::parse_schema;
use std::hint::black_box;

fn graph_building(c: &mut Criterion) {
    c.bench_function("graph_grafbase_many_plans", |b| {
        let supergraph_sdl =
            std::fs::read_to_string("./fixture/grafbase-many-plans/supergraph.graphql")
                .expect("Unable to read input file");
        let parsed_schema = parse_schema(&supergraph_sdl);
        let supergraph_state = SupergraphState::new(&parsed_schema);

        b.iter(|| {
            let graph = Graph::graph_from_supergraph_state(&supergraph_state)
                .expect("failed to create graph");
            black_box(graph);
        })
    });

    c.bench_function("graph_abstract_many_subgraphs", |b| {
        let supergraph_sdl =
            std::fs::read_to_string("./fixture/abstract-many-subgraphs/supergraph.graphql")
                .expect("Unable to read input file");
        let parsed_schema = parse_schema(&supergraph_sdl);
        let supergraph_state = SupergraphState::new(&parsed_schema);

        b.iter(|| {
            let graph = Graph::graph_from_supergraph_state(&supergraph_state)
                .expect("failed to create graph");
            black_box(graph);
        })
    });

    c.bench_function("graph_heavy_query", |b| {
        let supergraph_sdl = std::fs::read_to_string("./fixture/heavy-query/supergraph.graphql")
            .expect("Unable to read input file");
        let parsed_schema = parse_schema(&supergraph_sdl);
        let supergraph_state = SupergraphState::new(&parsed_schema);
        b.iter(|| {
            let graph = Graph::graph_from_supergraph_state(&supergraph_state)
                .expect("failed to create graph");
            black_box(graph);
        })
    });
}

fn all_benchmarks(c: &mut Criterion) {
    graph_building(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
