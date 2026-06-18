use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use graphql_tools::parser::query::Definition;
use hive_router_plan_executor::introspection::schema::{SchemaMetadata, SchemaWithMetadata};
use hive_router_plan_executor::projection::plan::FieldProjectionPlan;
use hive_router_plan_executor::projection::response::project_by_operation;
use hive_router_plan_executor::response::value::Value;
use hive_router_query_planner::{
    ast::{
        document::NormalizedDocument,
        normalization::{create_normalized_document, normalize_operation},
    },
    consumer_schema::ConsumerSchema,
    state::supergraph_state::SupergraphState,
    utils::parsing::{parse_operation, parse_schema},
};
use std::hint::black_box;
pub mod raw_result;

fn project_data_by_operation_test(c: &mut Criterion) {
    let operation_path = "../../bench/operation.graphql";
    let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
        .expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let planner = Box::leak(Box::new(
        hive_router_query_planner::planner::Planner::new_from_supergraph(
            &parsed_schema,
            Default::default(),
        )
        .expect("Failed to create planner from supergraph"),
    ));
    let parsed_document = parse_operation(
        &std::fs::read_to_string(operation_path).expect("Unable to read input file"),
    );
    let normalized_document = normalize_operation(&planner.supergraph, &parsed_document, None)
        .expect("Failed to normalize operation");
    let normalized_operation = normalized_document.executable_operation();
    let schema_metadata = &planner.consumer_schema.schema_metadata();
    let (root_type_name, projection_plan) =
        FieldProjectionPlan::from_operation(normalized_operation, schema_metadata);
    let result_as_string = raw_result::get_result_as_string();
    let projected_data_as_json: sonic_rs::Value =
        sonic_rs::from_slice(result_as_string.as_bytes()).unwrap();
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
                    &Default::default(),
                    bb_root_type_name,
                    &bb_projection_plan,
                    &None,
                    result_as_string.len(),
                    schema_metadata,
                )
                .unwrap();
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn abstract_interface_schema() -> String {
    let mut node_fields = String::from("id: ID!\n");
    for field_index in 0..10 {
        node_fields.push_str(&format!("f{field_index}: String!\n"));
    }

    let mut schema = format!("interface Node {{\n{node_fields}}}\n");
    for type_index in 0..12 {
        schema.push_str(&format!(
            "type Node{type_index} implements Node {{\n{node_fields}}}\n"
        ));
    }
    schema.push_str("type Query { nodes: [Node!]! }\n");
    schema
}

fn abstract_interface_operation() -> String {
    let mut operation = String::from("query AbstractProjection { nodes { __typename id");
    for field_index in 0..10 {
        operation.push_str(&format!(" f{field_index}"));
    }
    operation.push_str(" } }");
    operation
}

fn abstract_interface_response() -> String {
    let mut response = String::from(r#"{"__typename":"Query","nodes":["#);
    for row_index in 0..4000 {
        if row_index > 0 {
            response.push(',');
        }

        response.push_str(&format!(
            r#"{{"__typename":"Node{}","id":"id-{}""#,
            row_index % 12,
            row_index
        ));

        for field_index in 0..10 {
            response.push_str(&format!(
                r#","f{field_index}":"value-{row_index}-{field_index}""#
            ));
        }

        response.push('}');
    }
    response.push_str("]}");
    response
}

fn abstract_interface_projection_plan() -> (&'static str, Vec<FieldProjectionPlan>, SchemaMetadata)
{
    let supergraph = parse_schema(&abstract_interface_schema());
    let consumer_schema = ConsumerSchema::new_from_supergraph(&supergraph);
    let schema_metadata = consumer_schema.schema_metadata();

    let mut operation = parse_operation(&abstract_interface_operation());
    let operation_ast = operation
        .definitions
        .iter_mut()
        .find_map(|def| match def {
            Definition::Operation(op) => Some(op),
            _ => None,
        })
        .expect("operation definition");

    let supergraph_state = SupergraphState::new(&supergraph);
    let normalized_operation: NormalizedDocument = create_normalized_document(
        &supergraph_state,
        operation_ast.clone(),
        Some("AbstractProjection".into()),
    );
    let (root_type_name, projection_plan) =
        FieldProjectionPlan::from_operation(&normalized_operation.operation, &schema_metadata);

    (root_type_name, projection_plan, schema_metadata)
}

fn project_abstract_interface_data(c: &mut Criterion) {
    let (root_type_name, projection_plan, schema_metadata) = abstract_interface_projection_plan();
    let result_as_string = abstract_interface_response();
    let projected_data_as_json: sonic_rs::Value =
        sonic_rs::from_slice(result_as_string.as_bytes()).unwrap();

    c.bench_function("project_abstract_interface_data", |b| {
        b.iter_batched(
            || Value::from(projected_data_as_json.as_ref()),
            |data| {
                let result = project_by_operation(
                    &data,
                    vec![],
                    &Default::default(),
                    black_box(root_type_name),
                    black_box(&projection_plan),
                    &None,
                    result_as_string.len(),
                    &schema_metadata,
                )
                .unwrap();
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn all_benchmarks(c: &mut Criterion) {
    project_data_by_operation_test(c);
    project_abstract_interface_data(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
