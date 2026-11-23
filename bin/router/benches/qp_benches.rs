use criterion::{criterion_group, criterion_main, measurement::WallTime, Bencher, Criterion};
use graphql_tools::parser::query::Document;
use hive_router::query_planner::ast::minification::minify_operation;
use hive_router::query_planner::ast::normalization::normalize_operation;
use hive_router::query_planner::ast::operation::OperationDefinition;
use hive_router::query_planner::graph::PlannerOverrideContext;
use hive_router::query_planner::planner::{Planner, QueryPlannerOptions};
use hive_router::query_planner::state::supergraph_state::SupergraphState;
use hive_router::query_planner::utils::cancellation::CancellationToken;
use hive_router::query_planner::utils::parsing::{parse_operation, parse_schema};
use std::hint::black_box;
use std::time::Duration;

/// Entity reuse is off by default, but its planning is on here so its cost stays measured
fn planner_options() -> QueryPlannerOptions {
    QueryPlannerOptions {
        entity_reuse: true,
        ..Default::default()
    }
}

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

fn bench_query_plan(
    c: &mut Criterion,
    name: &str,
    planner: &Planner,
    operation: &OperationDefinition,
) {
    let cancellation_token = CancellationToken::new();

    // the router's own way to plan, fragment groups included
    let mut benchmark = |b: &mut Bencher<'_, WallTime>| {
        b.iter(|| {
            let query_plan = black_box(planner)
                .plan_from_normalized_operation(
                    black_box(operation),
                    PlannerOverrideContext::default(),
                    &cancellation_token,
                )
                .unwrap();
            black_box(query_plan);
        })
    };

    if matches!(
        name,
        "query_plan_wfg_mutation_1" | "query_plan_wfg_query_1" | "query_plan_wfg_query_2"
    ) {
        let mut group = c.benchmark_group(name);
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(5));
        group.bench_function("plan", &mut benchmark);
        group.finish();
    } else {
        c.bench_function(name, &mut benchmark);
    }
}

fn query_plan_pipeline(c: &mut Criterion) {
    let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
        .expect("Unable to read input file");
    let planner = Planner::new_from_supergraph(&parse_schema(&supergraph_sdl), planner_options())
        .expect("failed to create planner");
    let supergraph_state = &*planner.supergraph;

    let parsed_document = get_operation("../../bench/operation.graphql");
    let operation = get_executable_operation(&parsed_document, supergraph_state, Some("TestQuery"));
    bench_query_plan(c, "query_plan", &planner, &operation);

    bench_fixture_plans(
        c,
        &[
            (
                "query_plan_grafbase_many_plans",
                "./fixture/grafbase-many-plans/supergraph.graphql",
                "./fixture/grafbase-many-plans/operation.graphql",
                Some("ManyPlansQuery"),
            ),
            (
                "query_plan_bfg_operation_1",
                "./fixture/bfg/supergraph.graphql",
                "./fixture/bfg/operation-1.graphql",
                Some("Query1"),
            ),
            (
                "query_plan_bfg_operation_2",
                "./fixture/bfg/supergraph.graphql",
                "./fixture/bfg/operation-2.graphql",
                Some("Query2"),
            ),
            (
                "query_plan_wfg_mutation_1",
                "./fixture/wfg/supergraph.graphql",
                "./fixture/wfg/mutation-1.graphql",
                Some("Mutation1"),
            ),
            (
                "query_plan_wfg_query_1",
                "./fixture/wfg/supergraph.graphql",
                "./fixture/wfg/query-1.graphql",
                Some("Query1"),
            ),
            (
                "query_plan_wfg_query_2",
                "./fixture/wfg/supergraph.graphql",
                "./fixture/wfg/query-2.graphql",
                Some("Query2"),
            ),
        ],
    );

    c.bench_function("normalization", |b| {
        b.iter(|| {
            let op = get_executable_operation(
                black_box(&parsed_document),
                black_box(supergraph_state),
                black_box(Some("TestQuery")),
            );
            black_box(op);
        })
    });

    c.bench_function("minification", |b| {
        b.iter_batched(
            || operation.clone(),
            |cloned_operation| {
                let bb_supergraph_state = black_box(supergraph_state);
                let bb_operation = black_box(cloned_operation);
                let op = minify_operation(bb_operation, bb_supergraph_state).unwrap();
                black_box(op);
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

fn cache_preparation(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_preparation");
    for (name, schema_path, operation_path) in [
        (
            "bench",
            "../../bench/supergraph.graphql",
            "../../bench/operation.graphql",
        ),
        (
            "many_plans",
            "./fixture/grafbase-many-plans/supergraph.graphql",
            "./fixture/grafbase-many-plans/operation.graphql",
        ),
    ] {
        let schema = parse_schema(&std::fs::read_to_string(schema_path).unwrap());
        let planner = Planner::new_from_supergraph(&schema, planner_options()).unwrap();
        let document = get_operation(operation_path);
        let operation = get_executable_operation(&document, &planner.supergraph, None);
        let plan = planner
            .plan_from_normalized_operation(
                &operation,
                PlannerOverrideContext::default(),
                &CancellationToken::new(),
            )
            .unwrap();
        // Planning and cloning are setup. Only consuming the plan is measured.
        group.bench_function(name, |b| {
            b.iter_batched(
                || plan.clone(),
                |plan| black_box(plan.into_executable(&planner.supergraph)),
                criterion::BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

/// `@provides` under one union member, with fields from both members and a `@requires` that
/// reads the provided field.
fn provides_planning(c: &mut Criterion) {
    let schema = parse_schema(
        &std::fs::read_to_string("./fixture/tests/provides-union-member.supergraph.graphql")
            .unwrap(),
    );
    let planner = Planner::new_from_supergraph(&schema, planner_options()).unwrap();
    let document = parse_operation(
        "{ user { orders { label item { ... on Book { isbn stock } ... on Movie { title } } } } }",
    );
    let operation = get_executable_operation(&document, &planner.supergraph, None);
    bench_query_plan(c, "query_plan_provides_union", &planner, &operation);
}

/// Plans one operation per fixture, each bench named after the fixture
fn bench_fixture_plans(c: &mut Criterion, fixtures: &[(&str, &str, &str, Option<&str>)]) {
    for &(benchmark_name, schema_path, operation_path, operation_name) in fixtures {
        let supergraph_sdl =
            std::fs::read_to_string(schema_path).expect("Unable to read supergraph fixture");
        let fixture_planner =
            Planner::new_from_supergraph(&parse_schema(&supergraph_sdl), planner_options())
                .expect("failed to create planner");
        let fixture_document = get_operation(operation_path);
        let fixture_operation = get_executable_operation(
            &fixture_document,
            &fixture_planner.supergraph,
            operation_name,
        );

        bench_query_plan(c, benchmark_name, &fixture_planner, &fixture_operation);
    }
}

/// A single plan takes 0.5s or more here, so these run in their own group with 10 samples
/// instead of 100. Same bench names as before, unlike a benchmark_group.
fn slow_query_plans(c: &mut Criterion) {
    bench_fixture_plans(
        c,
        &[
            (
                "query_plan_abstract_many_subgraphs",
                "./fixture/abstract-many-subgraphs/supergraph.graphql",
                "./fixture/abstract-many-subgraphs/operation.graphql",
                Some("AbstractManySubgraphsBench"),
            ),
            (
                "query_plan_heavy_query",
                "./fixture/heavy-query/supergraph.graphql",
                "./fixture/heavy-query/operation.graphql",
                Some("TvpVod"),
            ),
        ],
    );
}

fn all_benchmarks(c: &mut Criterion) {
    let _ = tracing::subscriber::set_global_default(tracing::subscriber::NoSubscriber::default());

    query_plan_pipeline(c);
    cache_preparation(c);
    provides_planning(c);
}

criterion_group!(benches, all_benchmarks);
criterion_group! {
    name = slow_benches;
    config = Criterion::default().sample_size(10);
    targets = slow_query_plans
}
criterion_main!(benches, slow_benches);
