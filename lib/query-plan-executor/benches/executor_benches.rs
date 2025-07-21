#![recursion_limit = "256"]
use std::collections::HashMap;

use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use query_plan_executor::executors::common::SubgraphExecutor;
use query_plan_executor::executors::map::SubgraphExecutorMap;
use query_planner::ast::selection_item::SelectionItem;
use query_planner::ast::selection_set::InlineFragmentSelection;
use query_planner::graph::PlannerOverrideContext;
use query_planner::planner::plan_nodes::FlattenNodePathSegment;
use simd_json::BorrowedValue;
use std::hint::black_box;
mod non_projected_result_simd;

use query_plan_executor::schema_metadata::{SchemaMetadata, SchemaWithMetadata};
use query_plan_executor::{execute_query_plan, ExposeQueryPlanMode, ResponsesStorage};
use query_plan_executor::{ExecutableQueryPlan, ResponseValue};
use query_planner::ast::normalization::normalize_operation;
use query_planner::utils::parsing::parse_operation;
use query_planner::utils::parsing::parse_schema;
mod non_projected_result;
use serde_json::Value;
// This is a struct that tells Criterion.rs to use the "futures" crate's current-thread executor
use tokio::runtime::Runtime;

// fn query_plan_executor_pipeline_via_http(c: &mut Criterion) {
//     let rt = Runtime::new().expect("Failed to create Tokio runtime");
//     let operation_path = "../../bench/operation.graphql";
//     let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
//         .expect("Unable to read input file");
//     let parsed_schema = parse_schema(&supergraph_sdl);
//     let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
//         .expect("Failed to create planner from supergraph");
//     let parsed_document = parse_operation(
//         &std::fs::read_to_string(operation_path).expect("Unable to read input file"),
//     );
//     let normalized_document = normalize_operation(&planner.supergraph, &parsed_document, None)
//         .expect("Failed to normalize operation");
//     let normalized_operation = normalized_document.executable_operation();
//     let override_context = PlannerOverrideContext::default();
//     let query_plan = planner
//         .plan_from_normalized_operation(normalized_operation, override_context)
//         .expect("Failed to create query plan");
//     let subgraph_endpoint_map = planner.supergraph.subgraph_endpoint_map;
//     let schema_metadata = planner.consumer_schema.schema_metadata();
//     let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(subgraph_endpoint_map);
//     let (root_type_name, projection_selections) =
//         query_plan_executor::projection::FieldProjectionPlan::from_operation(
//             normalized_operation,
//             &schema_metadata,
//         );
//     c.bench_function("query_plan_executor_pipeline_via_http", |b| {
//         b.to_async(&rt).iter(|| async {
//             let query_plan = black_box(&query_plan);
//             let schema_metadata = black_box(&schema_metadata);
//             let subgraph_executor_map = black_box(&subgraph_executor_map);
//             let projection_selections = black_box(&projection_selections);
//             let root_type_name = black_box(root_type_name);
//             let has_introspection = false;
//             let result = execute_query_plan(
//                 query_plan,
//                 subgraph_executor_map,
//                 &None,
//                 schema_metadata,
//                 root_type_name,
//                 projection_selections,
//                 has_introspection,
//                 ExposeQueryPlanMode::No,
//             )
//             .await;
//             black_box(result)
//         });
//     });
// }

// fn query_plan_execution_without_projection_via_http(c: &mut Criterion) {
//     let rt = Runtime::new().expect("Failed to create Tokio runtime");
//     let operation_path = "../../bench/operation.graphql";
//     let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
//         .expect("Unable to read input file");
//     let parsed_schema = parse_schema(&supergraph_sdl);
//     let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
//         .expect("Failed to create planner from supergraph");
//     let parsed_document = parse_operation(
//         &std::fs::read_to_string(operation_path).expect("Unable to read input file"),
//     );
//     let normalized_document = normalize_operation(&planner.supergraph, &parsed_document, None)
//         .expect("Failed to normalize operation");
//     let normalized_operation = normalized_document.executable_operation();
//     let override_context = PlannerOverrideContext::default();
//     let query_plan = planner
//         .plan_from_normalized_operation(normalized_operation, override_context)
//         .expect("Failed to create query plan");
//     let subgraph_endpoint_map = planner.supergraph.subgraph_endpoint_map;
//     let schema_metadata = planner.consumer_schema.schema_metadata();
//     let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(subgraph_endpoint_map);
//     c.bench_function("query_plan_execution_without_projection_via_http", |b| {
//         b.to_async(&rt).iter(|| async {
//             let schema_metadata = black_box(&schema_metadata);
//             let subgraph_executor_map = black_box(&subgraph_executor_map);
//             let mut execution_context = query_plan_executor::QueryPlanExecutionContext {
//                 variable_values: &None,
//                 schema_metadata,
//                 subgraph_executor_map,
//                 errors: Vec::new(),
//                 extensions: HashMap::new(),
//             };
//             let query_plan = black_box(&query_plan);
//             let mut data = Value::Null;
//             let result = query_plan.execute(&mut execution_context, &mut data).await;
//             black_box(result);
//             black_box(data);
//         });
//     });
// }

// fn query_plan_executor_pipeline_locally(c: &mut Criterion) {
//     let rt = Runtime::new().expect("Failed to create Tokio runtime");
//     let operation_path = "../../bench/operation.graphql";
//     let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
//         .expect("Unable to read input file");
//     let parsed_schema = parse_schema(&supergraph_sdl);
//     let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
//         .expect("Failed to create planner from supergraph");
//     let parsed_document = parse_operation(
//         &std::fs::read_to_string(operation_path).expect("Unable to read input file"),
//     );
//     let normalized_document = normalize_operation(&planner.supergraph, &parsed_document, None)
//         .expect("Failed to normalize operation");
//     let normalized_operation = normalized_document.executable_operation();
//     let override_context = PlannerOverrideContext::default();
//     let query_plan = planner
//         .plan_from_normalized_operation(normalized_operation, override_context)
//         .expect("Failed to create query plan");
//     let schema_metadata = planner.consumer_schema.schema_metadata();
//     let mut subgraph_executor_map = SubgraphExecutorMap::new(); // No subgraphs in this testlet mut subgraph_executor_map = SubgraphExecutorMap::new(); // No subgraphs in this test
//     let accounts = subgraphs::accounts::get_subgraph();
//     let inventory = subgraphs::inventory::get_subgraph();
//     let products = subgraphs::products::get_subgraph();
//     let reviews = subgraphs::reviews::get_subgraph();
//     subgraph_executor_map.insert_boxed_arc("accounts".to_string(), accounts.to_boxed_arc());
//     subgraph_executor_map.insert_boxed_arc("inventory".to_string(), inventory.to_boxed_arc());
//     subgraph_executor_map.insert_boxed_arc("products".to_string(), products.to_boxed_arc());
//     subgraph_executor_map.insert_boxed_arc("reviews".to_string(), reviews.to_boxed_arc());

//     let (root_type_name, projection_selections) =
//         query_plan_executor::projection::FieldProjectionPlan::from_operation(
//             normalized_operation,
//             &schema_metadata,
//         );

//     c.bench_function("query_plan_executor_pipeline_locally", |b| {
//         b.to_async(&rt).iter(|| async {
//             let query_plan = black_box(&query_plan);
//             let schema_metadata = black_box(&schema_metadata);
//             let subgraph_executor_map = black_box(&subgraph_executor_map);
//             let projection_selections = black_box(&projection_selections);
//             let root_type_name = black_box(root_type_name);
//             let has_introspection = false;
//             let result = execute_query_plan(
//                 query_plan,
//                 subgraph_executor_map,
//                 &None,
//                 schema_metadata,
//                 root_type_name,
//                 projection_selections,
//                 has_introspection,
//                 ExposeQueryPlanMode::No,
//             )
//             .await;
//             black_box(result)
//         });
//     });
// }

// fn query_plan_executor_without_projection_locally(c: &mut Criterion) {
//     let rt = Runtime::new().expect("Failed to create Tokio runtime");
//     let operation_path = "../../bench/operation.graphql";
//     let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
//         .expect("Unable to read input file");
//     let parsed_schema = parse_schema(&supergraph_sdl);
//     let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
//         .expect("Failed to create planner from supergraph");
//     let parsed_document = parse_operation(
//         &std::fs::read_to_string(operation_path).expect("Unable to read input file"),
//     );
//     let normalized_document = normalize_operation(&planner.supergraph, &parsed_document, None)
//         .expect("Failed to normalize operation");
//     let normalized_operation = normalized_document.executable_operation();
//     let override_context = PlannerOverrideContext::default();
//     let query_plan = planner
//         .plan_from_normalized_operation(normalized_operation, override_context)
//         .expect("Failed to create query plan");
//     let schema_metadata = planner.consumer_schema.schema_metadata();
//     let mut subgraph_executor_map = SubgraphExecutorMap::new(); // No subgraphs in this testlet mut subgraph_executor_map = SubgraphExecutorMap::new(); // No subgraphs in this test
//     let accounts = subgraphs::accounts::get_subgraph();
//     let inventory = subgraphs::inventory::get_subgraph();
//     let products = subgraphs::products::get_subgraph();
//     let reviews = subgraphs::reviews::get_subgraph();
//     subgraph_executor_map.insert_boxed_arc("accounts".to_string(), accounts.to_boxed_arc());
//     subgraph_executor_map.insert_boxed_arc("inventory".to_string(), inventory.to_boxed_arc());
//     subgraph_executor_map.insert_boxed_arc("products".to_string(), products.to_boxed_arc());
//     subgraph_executor_map.insert_boxed_arc("reviews".to_string(), reviews.to_boxed_arc());

//     c.bench_function("query_plan_executor_without_projection_locally", |b| {
//         b.to_async(&rt).iter(|| async {
//             let query_plan = black_box(&query_plan);
//             let schema_metadata = black_box(&schema_metadata);
//             let subgraph_executor_map = black_box(&subgraph_executor_map);

//             let mut execution_context = query_plan_executor::QueryPlanExecutionContext {
//                 variable_values: &None,
//                 schema_metadata,
//                 subgraph_executor_map,
//                 errors: Vec::new(),
//                 extensions: HashMap::new(),
//             };
//             let query_plan = black_box(&query_plan);
//             let mut data = Value::Null;
//             let result = query_plan.execute(&mut execution_context, &mut data).await;
//             black_box(result);
//             black_box(data);
//         });
//     });
// }

fn project_data_by_operation(c: &mut Criterion) {
    let operation_path = "../../bench/operation.graphql";
    let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
        .expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
        .expect("Failed to create planner from supergraph");
    let parsed_document = parse_operation(
        &std::fs::read_to_string(operation_path).expect("Unable to read input file"),
    );
    let normalized_document = normalize_operation(&planner.supergraph, &parsed_document, None)
        .expect("Failed to normalize operation");
    let normalized_operation = normalized_document.executable_operation();
    let schema_metadata = planner.consumer_schema.schema_metadata();
    let (root_type_name, projection_selections) =
        query_plan_executor::projection::FieldProjectionPlan::from_operation(
            normalized_operation,
            &schema_metadata,
        );
    let mut projected_data_as_string = non_projected_result_simd::get_result_as_string();
    let projected_data_as_bytes = unsafe { projected_data_as_string.as_bytes_mut() };
    let projected_data_as_json: BorrowedValue =
        simd_json::from_slice(projected_data_as_bytes).unwrap();
    c.bench_function("project_data_by_operation", |b| {
        b.iter_batched(
            || {
                let val: ResponseValue = (&projected_data_as_json).into();
                val
            },
            |data| {
                let mut errors = vec![];
                let errors = black_box(&mut errors);
                let extensions = HashMap::new();
                let extensions = black_box(&extensions);
                let projection_selections = black_box(&projection_selections);
                let root_type_name = black_box(root_type_name);
                let result = query_plan_executor::projection::project_by_operation(
                    &data,
                    errors,
                    extensions,
                    root_type_name,
                    projection_selections,
                    &None,
                );
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

// fn traverse_and_collect(c: &mut Criterion) {
//     let path = [
//         FlattenNodePathSegment::Field("users".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("reviews".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("product".into()),
//         FlattenNodePathSegment::Field("reviews".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("author".into()),
//         FlattenNodePathSegment::Field("reviews".into()),
//         FlattenNodePathSegment::List,
//         FlattenNodePathSegment::Field("product".into()),
//     ];
//     let mut result: Value = non_projected_result::get_result();
//     let schema_metadata = SchemaMetadata::default();
//     c.bench_function("traverse_and_collect", |b| {
//         b.iter(|| {
//             let result = black_box(&mut result);
//             let schema_metadata = black_box(&schema_metadata);
//             let data = result.get_mut("data").unwrap();
//             let path = black_box(&path);
//             let mut results = vec![];
//             query_plan_executor::traverse_and_callback(data, path, schema_metadata, &mut |data| {
//                 results.push(data);
//             });
//             black_box(());
//             black_box(results);
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

//     c.bench_function("deep_merge_with_complex", |b| {
//         b.iter(|| {
//             let mut target = black_box(data_1.clone());
//             let source = black_box(data_2.clone());
//             query_plan_executor::deep_merge::deep_merge(&mut target, source);
//         });
//     });
// }

fn project_requires(c: &mut Criterion) {
    let path = [
        FlattenNodePathSegment::Field("users".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("reviews".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("product".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("reviews".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("author".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("reviews".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("product".into()),
    ];
    let mut result: Value = non_projected_result::get_result();
    let data = result.get_mut("data").unwrap();
    let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
        .expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
        .expect("Failed to create planner from supergraph");
    let schema_metadata = &planner.consumer_schema.schema_metadata();
    let mut representations = vec![];
    query_plan_executor::traverse_and_callback(data, &path, schema_metadata, &mut |data| {
        representations.push(data);
    });
    let mut representations: Vec<String> =
        representations.iter().map(|val| val.to_string()).collect();

    let representations: Vec<BorrowedValue> = representations
        .iter_mut() // Start with a mutable iterator over the strings.
        .map(|string| {
            // Each `string` is a `&mut String`.
            // Get a mutable byte slice inside the closure.
            let bytes = unsafe { string.as_bytes_mut() };

            // Parse the JSON from the byte slice.
            let json_value: BorrowedValue = simd_json::from_slice(bytes).unwrap();

            // Convert to the final type and return it.
            json_value
        })
        .collect();
    let final_representations: Vec<ResponseValue> = representations
        .iter() // Start with a mutable iterator over the strings.
        .map(|val| (val).into())
        .collect();

    // simd_json::from_slice(projected_data_as_bytes).unwrap();
    // .collect::<Vec<&mut [u8]>>();
    let subgraph_executor_map =
        SubgraphExecutorMap::from_http_endpoint_map(planner.supergraph.subgraph_endpoint_map);
    let execution_context = query_plan_executor::QueryPlanExecutionContext {
        variable_values: &None,
        subgraph_executor_map: &subgraph_executor_map,
        schema_metadata,
        errors: Vec::new(),
        extensions: HashMap::new(),
        response_storage: ResponsesStorage::new(),
    };
    let requires_selections: Vec<SelectionItem> =
        [SelectionItem::InlineFragment(InlineFragmentSelection {
            type_condition: "Product".to_string(),
            skip_if: None,
            include_if: None,
            selections: query_planner::ast::selection_set::SelectionSet {
                items: vec![
                    SelectionItem::Field(query_planner::ast::selection_set::FieldSelection {
                        name: "__typename".to_string(),
                        selections: query_planner::ast::selection_set::SelectionSet {
                            items: vec![],
                        },
                        alias: None,
                        arguments: None,
                        include_if: None,
                        skip_if: None,
                    }),
                    SelectionItem::Field(query_planner::ast::selection_set::FieldSelection {
                        name: "price".to_string(),
                        selections: query_planner::ast::selection_set::SelectionSet {
                            items: vec![],
                        },
                        alias: None,
                        arguments: None,
                        include_if: None,
                        skip_if: None,
                    }),
                    SelectionItem::Field(query_planner::ast::selection_set::FieldSelection {
                        name: "weight".to_string(),
                        selections: query_planner::ast::selection_set::SelectionSet {
                            items: vec![],
                        },
                        alias: None,
                        arguments: None,
                        include_if: None,
                        skip_if: None,
                    }),
                    SelectionItem::Field(query_planner::ast::selection_set::FieldSelection {
                        name: "upc".to_string(),
                        selections: query_planner::ast::selection_set::SelectionSet {
                            items: vec![],
                        },
                        alias: None,
                        arguments: None,
                        include_if: None,
                        skip_if: None,
                    }),
                ],
            },
        })]
        .to_vec();
    c.bench_function("project_requires", |b| {
        b.iter(|| {
            let execution_context = black_box(&execution_context);
            let mut buffer = String::with_capacity(1024);
            let mut first = true;
            for representation in black_box(&final_representations) {
                let requires = execution_context.project_requires(
                    &requires_selections,
                    representation,
                    &mut buffer,
                    first,
                    None,
                );
                if requires {
                    first = false;
                }
                black_box(requires);
            }
            black_box(buffer)
        });
    });
}

fn deep_merge_with_simple(c: &mut Criterion) {
    let mut data_1 = serde_json::json!({
        "users": []
    });

    let user_1 = serde_json::json!({
        "id": "1",
        "name": "Alice"
    });

    let mut data_2 = serde_json::json!({
        "users": []
    });
    let user_2 = serde_json::json!({
        "id": "1",
        "age": 30,
    });

    let data_1_users = data_1.get_mut("users").unwrap();
    let data_2_users = data_2.get_mut("users").unwrap();
    for _ in 0..30 {
        let user_1_clone = user_1.clone();
        let user_2_clone = user_2.clone();
        data_1_users.as_array_mut().unwrap().push(user_1_clone);
        data_2_users.as_array_mut().unwrap().push(user_2_clone);
    }

    c.bench_function("deep_merge_with_simple", |b| {
        b.iter(|| {
            let mut target = black_box(data_1.clone());
            let source = black_box(data_2.clone());
            query_plan_executor::deep_merge::deep_merge(&mut target, source);
        });
    });
}

fn all_benchmarks(c: &mut Criterion) {
    // query_plan_executor_without_projection_locally(c);
    // query_plan_executor_pipeline_locally(c);

    // query_plan_execution_without_projection_via_http(c);
    // query_plan_executor_pipeline_via_http(c);

    // deep_merge_with_simple(c);
    // deep_merge_with_complex(c);
    project_requires(c);
    // traverse_and_collect(c);
    project_data_by_operation(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
