// use bumpalo::Bump;
use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use executor::projection::response::project_by_operation;
use executor::response::value::Value;
use query_plan_executor::projection::FieldProjectionPlan;
use query_plan_executor::schema_metadata::SchemaWithMetadata;
// use executor::schema::metadata::SchemaMetadata;
use query_planner::ast::normalization::normalize_operation;
use query_planner::utils::parsing::{parse_operation, parse_schema};
// use simd_json::BorrowedValue;
use std::hint::black_box;
pub mod raw_result;

// fn sonic_into_ref(c: &mut Criterion) {
//     let projected_data_as_json: sonic_rs::Value =
//         sonic_rs::from_slice(raw_result::get_result_as_string().as_bytes()).unwrap();

//     c.bench_function("sonic_as_ref", |b| {
//         b.iter(|| {
//             let mut result = projected_data_as_json.as_ref();
//             let mut obj = sonic_rs::ValueRef::Null;
//             black_box(result);
//         });
//     });

//     let arena = Bump::new();
//     c.bench_function("sonic_into_ref", |b| {
//         b.iter(|| {
//             // let result: Value = projected_data_as_json.as_ref().into();
//             let result = Value::from(projected_data_as_json.as_ref());
//             black_box(result);
//         });
//     });

//     let mut projected_data_as_string = raw_result::get_result_as_string();
//     let projected_data_as_bytes = unsafe { projected_data_as_string.as_bytes_mut() };
//     let projected_data_as_json: BorrowedValue =
//         simd_json::from_slice(projected_data_as_bytes).unwrap();
//     // c.bench_function("simd_into_ref", |b| {
//     //     b.iter(|| {
//     //         let result: Value = (&projected_data_as_json).into();
//     //         black_box(result);
//     //     });
//     // });
//     // c.bench_function("simd_into_ref_value", |b| {
//     //     b.iter(|| {
//     //         let result: RefValue = (&projected_data_as_json).into();
//     //         black_box(result);
//     //     });
//     // });
// }

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
                let result =
                    project_by_operation(&data, bb_root_type_name, &bb_projection_plan, &None);
                black_box(result);
            },
            criterion::BatchSize::LargeInput,
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

// fn deep_merge_with_complex(c: &mut Criterion) {
//     let mut data_1 = serde_json::json!({
//         "users": []
//     });

//     let user_1 = serde_json::json!({
//         "id": "1",
//         "name": "Alice",
//         "reviews": []
//     });

//     let review_1 = serde_json::json!(
//     {
//         "id": "r1",
//         "content": "Great product!",
//         "product": {
//             "id": "p2",
//             "upc": "1234567890",
//         }
//     });

//     let user_2 = serde_json::json!({
//         "id": "1",
//         "age": 30,
//         "reviews": [],
//     });

//     let review_2 = serde_json::json!(
//     {
//         "id": "r1",
//         "product": {
//             "id": "p2",
//             "name": "Product B"
//         }
//     });

//     let mut data_2 = serde_json::json!({
//         "users": []
//     });

//     let data_1_users = data_1.get_mut("users").unwrap();
//     let data_2_users = data_2.get_mut("users").unwrap();
//     for _ in 0..30 {
//         let mut user_1_clone = user_1.clone();
//         let user_1_reviews = user_1_clone.get_mut("reviews").unwrap();
//         for _ in 0..5 {
//             let review_1_clone = review_1.clone();
//             user_1_reviews.as_array_mut().unwrap().push(review_1_clone);
//         }
//         data_1_users.as_array_mut().unwrap().push(user_1_clone);

//         let mut user_2_clone = user_2.clone();
//         let user_2_reviews = user_2_clone.get_mut("reviews").unwrap();
//         for _ in 0..5 {
//             let review_2_clone = review_2.clone();
//             user_2_reviews.as_array_mut().unwrap().push(review_2_clone);
//         }
//         data_2_users.as_array_mut().unwrap().push(user_2_clone);
//     }

//     let data_1_str = data_1.to_string();
//     let data_2_str = data_2.to_string();

//     let data_1_json: sonic_rs::Value = sonic_rs::from_str(&data_1_str).unwrap();
//     let data_2_json: sonic_rs::Value = sonic_rs::from_str(&data_2_str).unwrap();

//     let arena = Bump::new();
//     let data_1 = Value::from(data_1_json.as_ref());

//     c.bench_function("deep_merge_with_complex", |b| {
//         b.iter_batched(
//             || (data_1.clone()),
//             |mut target| {
//                 deep_merge(&mut target, data_2_json.as_ref());
//             },
//             BatchSize::SmallInput,
//         );
//     });
// }

fn all_benchmarks(c: &mut Criterion) {
    // project_requires_test(c);
    project_data_by_operation_test(c);
    // sonic_into_ref(c);
    // deep_merge_with_complex(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
