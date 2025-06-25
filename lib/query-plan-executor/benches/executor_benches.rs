#![recursion_limit = "256"]
use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use query_plan_executor::deep_merge::DeepMerge;
use query_plan_executor::executors::common::SubgraphExecutor;
use query_plan_executor::executors::map::SubgraphExecutorMap;
use query_plan_executor::nodes::query_plan_node::ExecutableQueryPlanNode;
use std::hint::black_box;

use query_plan_executor::schema_metadata::SchemaWithMetadata;
use query_planner::ast::normalization::normalize_operation;
use query_planner::utils::parsing::parse_operation;
use query_planner::utils::parsing::parse_schema;
mod non_projected_result;
// This is a struct that tells Criterion.rs to use the "futures" crate's current-thread executor
use tokio::runtime::Runtime;

fn query_plan_executor_pipeline_via_http(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create Tokio runtime");
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
    let query_plan = planner
        .plan_from_normalized_operation(normalized_operation)
        .expect("Failed to create query plan");
    let subgraph_endpoint_map = planner.supergraph.subgraph_endpoint_map;
    let schema_metadata = planner.consumer_schema.schema_metadata();
    let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(subgraph_endpoint_map);
    c.bench_function("query_plan_executor_pipeline_via_http", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let schema_metadata = black_box(&schema_metadata);
            let operation = black_box(&normalized_operation);
            let subgraph_executor_map = black_box(&subgraph_executor_map);
            let has_introspection = false;
            let result = query_plan
                .execute_operation(
                    subgraph_executor_map,
                    &None,
                    schema_metadata,
                    operation,
                    has_introspection,
                )
                .await;
            black_box(result)
        });
    });
}

fn query_plan_execution_without_projection_via_http(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create Tokio runtime");
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
    let query_plan = planner
        .plan_from_normalized_operation(normalized_operation)
        .expect("Failed to create query plan");
    let subgraph_endpoint_map = planner.supergraph.subgraph_endpoint_map;
    let schema_metadata = planner.consumer_schema.schema_metadata();
    let subgraph_executor_map = SubgraphExecutorMap::from_http_endpoint_map(subgraph_endpoint_map);
    c.bench_function("query_plan_execution_without_projection_via_http", |b| {
        b.to_async(&rt).iter(|| async {
            let schema_metadata = black_box(&schema_metadata);
            let subgraph_executor_map = black_box(&subgraph_executor_map);
            let execution_context = query_plan_executor::execution_context::ExecutionContext {
                variables: &None,
                schema_metadata,
                subgraph_executor_map,
            };
            let query_plan = black_box(&query_plan);
            let result = query_plan.execute(&execution_context).await;
            black_box(result);
        });
    });
}

fn query_plan_executor_pipeline_locally(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create Tokio runtime");
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
    let query_plan = planner
        .plan_from_normalized_operation(normalized_operation)
        .expect("Failed to create query plan");
    let schema_metadata = planner.consumer_schema.schema_metadata();
    let mut subgraph_executor_map = SubgraphExecutorMap::new(); // No subgraphs in this testlet mut subgraph_executor_map = SubgraphExecutorMap::new(); // No subgraphs in this test
    let accounts = subgraphs::accounts::get_subgraph();
    let inventory = subgraphs::inventory::get_subgraph();
    let products = subgraphs::products::get_subgraph();
    let reviews = subgraphs::reviews::get_subgraph();
    subgraph_executor_map.insert_boxed_arc("accounts".to_string(), accounts.to_boxed_arc());
    subgraph_executor_map.insert_boxed_arc("inventory".to_string(), inventory.to_boxed_arc());
    subgraph_executor_map.insert_boxed_arc("products".to_string(), products.to_boxed_arc());
    subgraph_executor_map.insert_boxed_arc("reviews".to_string(), reviews.to_boxed_arc());

    c.bench_function("query_plan_executor_pipeline_locally", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let schema_metadata = black_box(&schema_metadata);
            let operation = black_box(&normalized_operation);
            let subgraph_executor_map = black_box(&subgraph_executor_map);
            let has_introspection = false;
            let result = query_plan
                .execute_operation(
                    subgraph_executor_map,
                    &None,
                    schema_metadata,
                    operation,
                    has_introspection,
                )
                .await;
            black_box(result)
        });
    });
}

fn query_plan_executor_without_projection_locally(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create Tokio runtime");
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
    let query_plan = planner
        .plan_from_normalized_operation(normalized_operation)
        .expect("Failed to create query plan");
    let schema_metadata = planner.consumer_schema.schema_metadata();
    let mut subgraph_executor_map = SubgraphExecutorMap::new(); // No subgraphs in this testlet mut subgraph_executor_map = SubgraphExecutorMap::new(); // No subgraphs in this test
    let accounts = subgraphs::accounts::get_subgraph();
    let inventory = subgraphs::inventory::get_subgraph();
    let products = subgraphs::products::get_subgraph();
    let reviews = subgraphs::reviews::get_subgraph();
    subgraph_executor_map.insert_boxed_arc("accounts".to_string(), accounts.to_boxed_arc());
    subgraph_executor_map.insert_boxed_arc("inventory".to_string(), inventory.to_boxed_arc());
    subgraph_executor_map.insert_boxed_arc("products".to_string(), products.to_boxed_arc());
    subgraph_executor_map.insert_boxed_arc("reviews".to_string(), reviews.to_boxed_arc());

    c.bench_function("query_plan_executor_without_projection_locally", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let schema_metadata = black_box(&schema_metadata);
            let subgraph_executor_map = black_box(&subgraph_executor_map);

            let ctx = query_plan_executor::execution_context::ExecutionContext {
                variables: &None,
                schema_metadata,
                subgraph_executor_map,
            };
            let query_plan = black_box(&query_plan);
            let result = query_plan.execute(&ctx).await;
            black_box(result);
        });
    });
}

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
    let operation = black_box(&normalized_operation);
    c.bench_function("project_data_by_operation", |b| {
        b.iter(|| {
            let mut data = non_projected_result::get_result().clone();
            let data = black_box(&mut data);
            let mut errors = vec![];
            let errors = black_box(&mut errors);
            let operation = black_box(&operation);
            let schema_metadata = black_box(&schema_metadata);
            query_plan_executor::projection::project_data_by_operation(
                data,
                errors,
                operation,
                schema_metadata,
                &None,
            );
            black_box(());
        });
    });
}

fn deep_merge_with_complex(c: &mut Criterion) {
    let mut data_1 = serde_json::json!({
        "users": []
    });

    let user_1 = serde_json::json!({
        "id": "1",
        "name": "Alice",
        "reviews": []
    });

    let review_1 = serde_json::json!(
    {
        "id": "r1",
        "content": "Great product!",
        "product": {
            "id": "p2",
            "upc": "1234567890",
        }
    });

    let user_2 = serde_json::json!({
        "id": "1",
        "age": 30,
        "reviews": [],
    });

    let review_2 = serde_json::json!(
    {
        "id": "r1",
        "product": {
            "id": "p2",
            "name": "Product B"
        }
    });

    let mut data_2 = serde_json::json!({
        "users": []
    });

    let data_1_users = data_1.get_mut("users").unwrap();
    let data_2_users = data_2.get_mut("users").unwrap();
    for _ in 0..30 {
        let mut user_1_clone = user_1.clone();
        let user_1_reviews = user_1_clone.get_mut("reviews").unwrap();
        for _ in 0..5 {
            let review_1_clone = review_1.clone();
            user_1_reviews.as_array_mut().unwrap().push(review_1_clone);
        }
        data_1_users.as_array_mut().unwrap().push(user_1_clone);

        let mut user_2_clone = user_2.clone();
        let user_2_reviews = user_2_clone.get_mut("reviews").unwrap();
        for _ in 0..5 {
            let review_2_clone = review_2.clone();
            user_2_reviews.as_array_mut().unwrap().push(review_2_clone);
        }
        data_2_users.as_array_mut().unwrap().push(user_2_clone);
    }

    c.bench_function("deep_merge_with_complex", |b| {
        b.iter(|| {
            let mut target = black_box(data_1.clone());
            let source = black_box(data_2.clone());
            target.deep_merge(source);
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
            target.deep_merge(source);
        });
    });
}

fn all_benchmarks(c: &mut Criterion) {
    deep_merge_with_simple(c);
    deep_merge_with_complex(c);
    project_data_by_operation(c);
    query_plan_executor_without_projection_locally(c);
    query_plan_executor_pipeline_locally(c);

    query_plan_execution_without_projection_via_http(c);
    query_plan_executor_pipeline_via_http(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
