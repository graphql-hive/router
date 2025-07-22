use executor::projection::plan::ProjectionPlan;
use executor::projection::response::project_by_operation;
use executor::response::value::Value;
use executor::schema::metadata::SchemaMetadata;
use query_planner::ast::normalization::normalize_operation;
use query_planner::utils::parsing::{parse_operation, parse_schema};
use simd_json::BorrowedValue;
mod raw_result;

fn main() {
    let operation_path = "./bench/operation.graphql";
    let supergraph_sdl =
        std::fs::read_to_string("./bench/supergraph.graphql").expect("Unable to read input file");
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
    let schema_metadata = SchemaMetadata::new(&planner.consumer_schema);
    let (root_type_name, projection_plan) =
        ProjectionPlan::from_operation(normalized_operation, &schema_metadata);
    let mut projected_data_as_string = raw_result::get_result_as_string();
    let projected_data_as_bytes = unsafe { projected_data_as_string.as_bytes_mut() };
    let projected_data_as_json: BorrowedValue =
        simd_json::from_slice(projected_data_as_bytes).unwrap();

    let val: Value = (&projected_data_as_json).into();

    for _ in 0..100_000_000 {
        let result = project_by_operation(
            &val,
            root_type_name,
            &projection_plan.root_selections,
            &None,
        );
        println!("{}", result.len())
    }
}
