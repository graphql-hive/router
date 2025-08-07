use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use executor::projection::response::project_by_operation;
use executor::response::value::Value;
use query_plan_executor::projection::FieldProjectionPlan;
use query_plan_executor::schema_metadata::SchemaWithMetadata;
use query_planner::ast::normalization::normalize_operation;
use query_planner::utils::parsing::{parse_operation, parse_schema};
use std::hint::black_box;
pub mod raw_result;

fn project_data_by_operation_test(c: &mut Criterion) {
    let operation_path = "../../bench/operation.graphql";
    let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
        .expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let planner = Box::leak(Box::new(
        query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
            .expect("Failed to create planner from supergraph"),
    ));
    let parsed_document = parse_operation(
        &std::fs::read_to_string(operation_path).expect("Unable to read input file"),
    );
    let normalized_document = normalize_operation(&planner.supergraph, &parsed_document, None)
        .expect("Failed to normalize operation");
    let normalized_operation = normalized_document.executable_operation();
    let (root_type_name, projection_plan) = FieldProjectionPlan::from_operation(
        normalized_operation,
        &planner.consumer_schema.schema_metadata(),
    );
    let projected_data_as_json: sonic_rs::Value =
        sonic_rs::from_slice(raw_result::get_result_as_string().as_bytes()).unwrap();
    c.bench_function("project_data_by_operation", |b| {
        b.iter_batched(
            || {
                let val: Value = Value::from(projected_data_as_json.as_ref());
                val
            },
            |data| {
                let bb_projection_plan = black_box(&projection_plan);
                let bb_root_type_name = black_box(root_type_name);
                let result = project_by_operation(
                    &data,
                    vec![],
                    &None,
                    bb_root_type_name,
                    &bb_projection_plan,
                    &None,
                );
                black_box(result);
            },
            criterion::BatchSize::LargeInput,
        );
    });
}

fn all_benchmarks(c: &mut Criterion) {
    project_data_by_operation_test(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
