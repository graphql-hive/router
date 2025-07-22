#![recursion_limit = "256"]
use std::collections::HashMap;

use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use executor::projection::plan::{FieldProjectionPlan, ProjectionPlan};
use executor::projection::response::project_by_operation;
use executor::response::value::Value;
use executor::schema::metadata::SchemaMetadata;
use query_planner::ast::normalization::normalize_operation;
use query_planner::ast::selection_item::SelectionItem;
use query_planner::ast::selection_set::InlineFragmentSelection;
use query_planner::planner::plan_nodes::FlattenNodePathSegment;
use query_planner::utils::parsing::{parse_operation, parse_schema};
use simd_json::derived::ValueObjectAccess;
use simd_json::BorrowedValue;
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
    let schema_metadata = SchemaMetadata::new(&planner.consumer_schema);
    let (root_type_name, projection_plan) =
        ProjectionPlan::from_operation(normalized_operation, &schema_metadata);
    let mut projected_data_as_string = raw_result::get_result_as_string();
    let projected_data_as_bytes = unsafe { projected_data_as_string.as_bytes_mut() };
    let projected_data_as_json: BorrowedValue =
        simd_json::from_slice(projected_data_as_bytes).unwrap();
    c.bench_function("project_data_by_operation", |b| {
        b.iter_batched(
            || {
                let val: Value = (&projected_data_as_json).into();
                val
            },
            |data| {
                let bb_projection_plan = black_box(&projection_plan);
                let bb_root_type_name = black_box(root_type_name);
                let result = project_by_operation(
                    &data,
                    bb_root_type_name,
                    &bb_projection_plan.root_selections,
                    &None,
                );
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

// fn project_requires_test(c: &mut Criterion) {
//     let path = [
//         FlattenNodePathSegment::Field("users".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("reviews".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("product".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("reviews".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("author".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("reviews".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("product".into()),
//     ];
//     let mut projected_data_as_string = raw_result::get_result_as_string();
//     let projected_data_as_bytes = unsafe { projected_data_as_string.as_bytes_mut() };
//     let projected_data_as_json: BorrowedValue =
//         simd_json::from_slice(projected_data_as_bytes).unwrap();
//     let mut data: ResponseValue = projected_data_as_json
//         .get("data")
//         .unwrap()
//         .clone() // Clone the borrowed value into an owned one
//         .into();
//     let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
//         .expect("Unable to read input file");
//     let parsed_schema = parse_schema(&supergraph_sdl);
//     let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
//         .expect("Failed to create planner from supergraph");
//     let schema_metadata = &planner.consumer_schema.schema_metadata();
//     let mut representations = vec![];
//     query_plan_executor::traverse_and_callback(&mut data, &path, schema_metadata, &mut |data| {
//         representations.push(data);
//     });

//     let subgraph_executor_map =
//         SubgraphExecutorMap::from_http_endpoint_map(planner.supergraph.subgraph_endpoint_map);
//     let execution_context = query_plan_executor::QueryPlanExecutionContext {
//         variable_values: &None,
//         subgraph_executor_map: &subgraph_executor_map,
//         schema_metadata,
//         errors: Vec::new(),
//         extensions: HashMap::new(),
//         response_storage: ResponsesStorage::new(),
//     };
//     let requires_selections: Vec<SelectionItem> =
//         [SelectionItem::InlineFragment(InlineFragmentSelection {
//             type_condition: "Product".to_string(),
//             skip_if: None,
//             include_if: None,
//             selections: query_planner::ast::selection_set::SelectionSet {
//                 items: vec![
//                     SelectionItem::Field(query_planner::ast::selection_set::FieldSelection {
//                         name: "__typename".to_string(),
//                         selections: query_planner::ast::selection_set::SelectionSet {
//                             items: vec![],
//                         },
//                         alias: None,
//                         arguments: None,
//                         include_if: None,
//                         skip_if: None,
//                     }),
//                     SelectionItem::Field(query_planner::ast::selection_set::FieldSelection {
//                         name: "price".to_string(),
//                         selections: query_planner::ast::selection_set::SelectionSet {
//                             items: vec![],
//                         },
//                         alias: None,
//                         arguments: None,
//                         include_if: None,
//                         skip_if: None,
//                     }),
//                     SelectionItem::Field(query_planner::ast::selection_set::FieldSelection {
//                         name: "weight".to_string(),
//                         selections: query_planner::ast::selection_set::SelectionSet {
//                             items: vec![],
//                         },
//                         alias: None,
//                         arguments: None,
//                         include_if: None,
//                         skip_if: None,
//                     }),
//                     SelectionItem::Field(query_planner::ast::selection_set::FieldSelection {
//                         name: "upc".to_string(),
//                         selections: query_planner::ast::selection_set::SelectionSet {
//                             items: vec![],
//                         },
//                         alias: None,
//                         arguments: None,
//                         include_if: None,
//                         skip_if: None,
//                     }),
//                 ],
//             },
//         })]
//         .to_vec();
//     c.bench_function("project_requires", |b| {
//         b.iter(|| {
//             let execution_context = black_box(&execution_context);
//             let mut buffer = String::with_capacity(1024);
//             let mut first = true;
//             for representation in black_box(&representations) {
//                 let requires = execution_context.project_requires(
//                     &requires_selections,
//                     representation,
//                     &mut buffer,
//                     first,
//                     None,
//                 );
//                 if requires {
//                     first = false;
//                 }
//                 black_box(requires);
//             }
//             black_box(buffer)
//         });
//     });
// }

fn all_benchmarks(c: &mut Criterion) {
    // project_requires_test(c);
    project_data_by_operation_test(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
