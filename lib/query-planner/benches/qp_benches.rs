use criterion::{criterion_group, criterion_main, Criterion};
use graphql_parser::query::Document;
use hive_router_query_planner::ast::minification::minify_operation;
use hive_router_query_planner::ast::normalization::normalize_operation;
use hive_router_query_planner::ast::operation::OperationDefinition;
use hive_router_query_planner::graph::Graph;
use hive_router_query_planner::graph::PlannerOverrideContext;
use hive_router_query_planner::planner::best::find_best_combination;
use hive_router_query_planner::planner::fetch::fetch_graph::build_fetch_graph_from_query_tree;
use hive_router_query_planner::planner::query_plan::build_query_plan_from_fetch_graph;
use hive_router_query_planner::planner::walker::walk_operation;
use hive_router_query_planner::state::supergraph_state::SupergraphState;
use hive_router_query_planner::utils::cancellation::CancellationToken;
use hive_router_query_planner::utils::parsing::{parse_operation, parse_schema};
use std::hint::black_box;

fn get_operation(operation_path: &str) -> Document<'static, String> {
    let document_text = std::fs::read_to_string(operation_path).expect("Unable to read input file");
    parse_operation(&document_text)
}

fn get_executable_operation(
    parsed_document: &Document<'static, String>,
    supergraph_state: &SupergraphState,
    operation_name: Option<&str>,
) -> OperationDefinition {
    normalize_operation(supergraph_state, parsed_document, operation_name)
        .unwrap()
        .operation
}

fn query_plan_pipeline(c: &mut Criterion) {
    let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
        .expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let supergraph_state = SupergraphState::new(&parsed_schema);
    let graph =
        Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph");

    let parsed_document = get_operation("../../bench/operation.graphql");
    let operation =
        get_executable_operation(&parsed_document, &supergraph_state, Some("TestQuery"));
    let override_context = PlannerOverrideContext::default();
    let cancellation_token = CancellationToken::new();

    c.bench_function("query_plan", |b| {
        b.iter(|| {
            let bb_graph = black_box(&graph);
            let bb_operation = black_box(&operation);
            let bb_supergraph_state = black_box(&supergraph_state);
            let bb_override_context = black_box(&override_context);

            let best_paths_per_leaf = walk_operation(
                bb_graph,
                bb_supergraph_state,
                bb_override_context,
                bb_operation,
                &cancellation_token,
            )
            .expect("walk_operation failed during benchmark");
            let query_tree =
                find_best_combination(bb_graph, best_paths_per_leaf, &cancellation_token).unwrap();
            let fetch_graph = build_fetch_graph_from_query_tree(
                bb_graph,
                bb_supergraph_state,
                bb_override_context,
                query_tree,
                &cancellation_token,
            )
            .unwrap();
            let query_plan = build_query_plan_from_fetch_graph(
                fetch_graph,
                bb_supergraph_state,
                &cancellation_token,
            )
            .unwrap();
            black_box(query_plan);
        })
    });

    c.bench_function("query_plan_grafbase_many_plans", |b| {
        let supergraph_sdl =
            std::fs::read_to_string("./fixture/grafbase-many-plans/supergraph.graphql")
                .expect("Unable to read input file");
        let parsed_schema = parse_schema(&supergraph_sdl);
        let supergraph_state = SupergraphState::new(&parsed_schema);
        let graph =
            Graph::graph_from_supergraph_state(&supergraph_state).expect("failed to create graph");

        let parsed_document = get_operation("./fixture/grafbase-many-plans/operation.graphql");
        let operation =
            get_executable_operation(&parsed_document, &supergraph_state, Some("ManyPlansQuery"));
        let override_context = PlannerOverrideContext::default();

        b.iter(|| {
            let bb_graph = black_box(&graph);
            let bb_operation = black_box(&operation);
            let bb_supergraph_state = black_box(&supergraph_state);
            let bb_override_context = black_box(&override_context);

            let best_paths_per_leaf = walk_operation(
                bb_graph,
                bb_supergraph_state,
                bb_override_context,
                bb_operation,
                &cancellation_token,
            )
            .expect("walk_operation failed during benchmark");
            let query_tree =
                find_best_combination(bb_graph, best_paths_per_leaf, &cancellation_token).unwrap();
            let fetch_graph = build_fetch_graph_from_query_tree(
                bb_graph,
                bb_supergraph_state,
                bb_override_context,
                query_tree,
                &cancellation_token,
            )
            .unwrap();
            let query_plan = build_query_plan_from_fetch_graph(
                fetch_graph,
                bb_supergraph_state,
                &cancellation_token,
            )
            .unwrap();
            black_box(query_plan);
        })
    });

    c.bench_function("normalization", |b| {
        b.iter(|| {
            let op = get_executable_operation(
                black_box(&parsed_document),
                black_box(&supergraph_state),
                black_box(Some("TestQuery")),
            );
            black_box(op);
        })
    });

    c.bench_function("minification", |b| {
        b.iter_batched(
            || operation.clone(),
            |cloned_operation| {
                let bb_supergraph_state = black_box(&supergraph_state);
                let bb_operation = black_box(cloned_operation);
                let op = minify_operation(bb_operation, bb_supergraph_state).unwrap();
                black_box(op);
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

fn all_benchmarks(c: &mut Criterion) {
    query_plan_pipeline(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
