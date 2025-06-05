use criterion::{black_box, criterion_group, criterion_main, Criterion};

use graphql_parser::query::Document;
use query_planner::ast::normalization::normalize_operation;
use query_planner::ast::operation::OperationDefinition;
use query_planner::consumer_schema::ConsumerSchema;
use query_planner::graph::Graph;
use query_planner::planner::fetch::fetch_graph::build_fetch_graph_from_query_tree;
use query_planner::planner::query_plan::build_query_plan_from_fetch_graph;
use query_planner::planner::tree::paths_to_trees;
use query_planner::planner::tree::query_tree::QueryTree;
use query_planner::planner::walker::walk_operation;
use query_planner::state::supergraph_state::SupergraphState;
use query_planner::utils::parsing::{parse_operation, parse_schema};

fn get_operation(operation_path: &str) -> Document<'static, String> {
    let document_text = std::fs::read_to_string(operation_path).expect("Unable to read input file");
    parse_operation(&document_text)
}

fn get_executable_operation(
    parsed_document: &Document<'static, String>,
    consumer_schema: &ConsumerSchema,
    operation_name: Option<&str>,
) -> OperationDefinition {
    normalize_operation(consumer_schema, parsed_document, operation_name).operation
}

fn get_supergraph(path: &str) -> (Graph, ConsumerSchema) {
    let supergraph_sdl = std::fs::read_to_string(path).expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let supergraph_state = SupergraphState::new(&parsed_schema);

    (
        Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph"),
        ConsumerSchema::new_from_supergraph(&parsed_schema),
    )
}

fn query_plan_pipeline(c: &mut Criterion) {
    let (graph, consumer_schema) = get_supergraph("../../bench/supergraph.graphql");
    let parsed_document = get_operation("../../bench/operation.graphql");
    let operation = get_executable_operation(&parsed_document, &consumer_schema, Some("TestQuery"));

    c.bench_function("query_plan", |b| {
        b.iter(|| {
            let best_paths_per_leaf = walk_operation(black_box(&graph), black_box(&operation))
                .expect("walk_operation failed during benchmark");
            let qtps = paths_to_trees(black_box(&graph), black_box(&best_paths_per_leaf)).unwrap();
            let query_tree = QueryTree::merge_trees(black_box(qtps));
            let fetch_graph =
                build_fetch_graph_from_query_tree(black_box(&graph), black_box(query_tree))
                    .unwrap();
            let query_plan = build_query_plan_from_fetch_graph(black_box(fetch_graph)).unwrap();
            black_box(query_plan);
        })
    });

    c.bench_function("normalization", |b| {
        b.iter(|| {
            let op =
                get_executable_operation(&parsed_document, &consumer_schema, Some("TestQuery"));
            black_box(op);
        })
    });
}

fn all_benchmarks(c: &mut Criterion) {
    query_plan_pipeline(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
