#![recursion_limit = "256"]
use std::collections::HashMap;
use std::sync::Arc;

use criterion::black_box;
use criterion::Criterion;
use criterion::{criterion_group, criterion_main};

use query_plan_executor::execute_query_plan;
use query_plan_executor::execute_query_plan_with_http_executor;
use query_plan_executor::executors::http::HTTPSubgraphExecutor;
use query_plan_executor::schema_metadata::SchemaWithMetadata;
use query_plan_executor::ExecutableQueryPlan;
use query_planner::ast::normalization::normalize_operation;
use query_planner::utils::parsing::parse_operation;
use query_planner::utils::parsing::parse_schema;
mod non_projected_result;
use serde_json::Value;
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
    let http_client = reqwest::Client::new();
    c.bench_function("query_plan_executor_pipeline_via_http", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let subgraph_endpoint_map = black_box(&subgraph_endpoint_map);
            let schema_metadata = black_box(&schema_metadata);
            let operation = black_box(&normalized_operation);
            let has_introspection = false;
            let http_client = black_box(&http_client);
            let result = execute_query_plan_with_http_executor(
                query_plan,
                subgraph_endpoint_map,
                &None,
                schema_metadata,
                operation,
                has_introspection,
                http_client,
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
    let http_client = reqwest::Client::new();
    c.bench_function("query_plan_execution_without_projection_via_http", |b| {
        b.to_async(&rt).iter(|| async {
            let schema_metadata = black_box(&schema_metadata);
            let subgraph_endpoint_map = black_box(&subgraph_endpoint_map);
            let http_client = black_box(&http_client);
            let executor = HTTPSubgraphExecutor {
                subgraph_endpoint_map,
                http_client,
            };
            let executor = Arc::new(executor);
            let mut execution_context = query_plan_executor::QueryPlanExecutionContext {
                variable_values: &None,
                schema_metadata,
                executor,
                errors: Vec::new(),
                extensions: HashMap::new(),
            };
            let query_plan = black_box(&query_plan);
            let mut data = Value::Null;
            let result = query_plan.execute(&mut execution_context, &mut data).await;
            black_box(result);
            black_box(data);
        });
    });
}

// TODO: Use LocalExecutor later
struct TestExecutor {
    accounts: async_graphql::Schema<
        subgraphs::accounts::Query,
        async_graphql::EmptyMutation,
        async_graphql::EmptySubscription,
    >,
    inventory: async_graphql::Schema<
        subgraphs::inventory::Query,
        async_graphql::EmptyMutation,
        async_graphql::EmptySubscription,
    >,
    products: async_graphql::Schema<
        subgraphs::products::Query,
        async_graphql::EmptyMutation,
        async_graphql::EmptySubscription,
    >,
    reviews: async_graphql::Schema<
        subgraphs::reviews::Query,
        async_graphql::EmptyMutation,
        async_graphql::EmptySubscription,
    >,
}

#[async_trait::async_trait]
impl query_plan_executor::executors::common::SubgraphExecutor for TestExecutor {
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: query_plan_executor::ExecutionRequest,
    ) -> query_plan_executor::ExecutionResult {
        match subgraph_name {
            "accounts" => self.accounts.execute(execution_request).await.into(),
            "inventory" => self.inventory.execute(execution_request).await.into(),
            "products" => self.products.execute(execution_request).await.into(),
            "reviews" => self.reviews.execute(execution_request).await.into(),
            _ => query_plan_executor::ExecutionResult::from_error_message(format!(
                "Subgraph {} not found in schema map",
                subgraph_name
            )),
        }
    }
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
    let executor = TestExecutor {
        accounts: subgraphs::accounts::get_subgraph(),
        inventory: subgraphs::inventory::get_subgraph(),
        products: subgraphs::products::get_subgraph(),
        reviews: subgraphs::reviews::get_subgraph(),
    };
    let executor = Arc::new(executor);
    c.bench_function("query_plan_executor_pipeline_locally", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let schema_metadata = black_box(&schema_metadata);
            let operation = black_box(&normalized_operation);
            let executor = black_box(executor.clone());
            let has_introspection = false;
            let result = execute_query_plan(
                query_plan,
                executor,
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
    let executor = TestExecutor {
        accounts: subgraphs::accounts::get_subgraph(),
        inventory: subgraphs::inventory::get_subgraph(),
        products: subgraphs::products::get_subgraph(),
        reviews: subgraphs::reviews::get_subgraph(),
    };
    let executor = Arc::new(executor);
    c.bench_function("query_plan_executor_without_projection_locally", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let schema_metadata = black_box(&schema_metadata);
            let executor = black_box(executor.clone());

            let mut execution_context = query_plan_executor::QueryPlanExecutionContext {
                variable_values: &None,
                schema_metadata,
                executor,
                errors: Vec::new(),
                extensions: HashMap::new(),
            };
            let query_plan = black_box(&query_plan);
            let mut data = Value::Null;
            let result = query_plan.execute(&mut execution_context, &mut data).await;
            black_box(result);
            black_box(data);
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
            let mut data = non_projected_result::get_result();
            let data = black_box(&mut data);
            let operation = black_box(&operation);
            let schema_metadata = black_box(&schema_metadata);
            query_plan_executor::project_data_by_operation(
                data,
                &mut vec![],
                operation,
                schema_metadata,
                &None,
            );
            black_box(());
        });
    });
}

fn all_benchmarks(c: &mut Criterion) {
    query_plan_execution_without_projection_via_http(c);
    query_plan_executor_pipeline_via_http(c);

    query_plan_executor_without_projection_locally(c);
    query_plan_executor_pipeline_locally(c);

    project_data_by_operation(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
