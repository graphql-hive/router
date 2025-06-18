#![recursion_limit = "256"]
use std::collections::HashMap;

use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use query_planner::ast::selection_item::SelectionItem;
use query_planner::ast::selection_set::InlineFragmentSelection;
use std::hint::black_box;

use query_plan_executor::execute_query_plan;
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
    let executor = HTTPSubgraphExecutor::new(subgraph_endpoint_map);
    c.bench_function("query_plan_executor_pipeline_via_http", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let schema_metadata = black_box(&schema_metadata);
            let operation = black_box(&normalized_operation);
            let executor = black_box(&executor);
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
    let executor = HTTPSubgraphExecutor::new(subgraph_endpoint_map);
    c.bench_function("query_plan_execution_without_projection_via_http", |b| {
        b.to_async(&rt).iter(|| async {
            let schema_metadata = black_box(&schema_metadata);
            let executor = black_box(&executor);
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
    c.bench_function("query_plan_executor_pipeline_locally", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let schema_metadata = black_box(&schema_metadata);
            let operation = black_box(&normalized_operation);
            let executor = black_box(&executor);
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
    c.bench_function("query_plan_executor_without_projection_locally", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let schema_metadata = black_box(&schema_metadata);
            let executor = black_box(&executor);

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
            let mut errors = vec![];
            let errors = black_box(&mut errors);
            let operation = black_box(&operation);
            let schema_metadata = black_box(&schema_metadata);
            query_plan_executor::project_data_by_operation(
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

fn traverse_and_collect(c: &mut Criterion) {
    let path = [
        "users", "@", "reviews", "@", "product", "reviews", "@", "author", "reviews", "@",
        "product",
    ];
    let mut result: Value = non_projected_result::get_result();
    c.bench_function("traverse_and_collect", |b| {
        b.iter(|| {
            let result = black_box(&mut result);
            let data = result.get_mut("data").unwrap();
            let path = black_box(&path);
            let result = query_plan_executor::traverse_and_collect(data, path);
            black_box(result);
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
            query_plan_executor::deep_merge::deep_merge(&mut target, source);
        });
    });
}

fn project_requires(c: &mut Criterion) {
    let path = [
        "users", "@", "reviews", "@", "product", "reviews", "@", "author", "reviews", "@",
        "product",
    ];
    let mut result: Value = non_projected_result::get_result();
    let data = result.get_mut("data").unwrap();
    let representations = query_plan_executor::traverse_and_collect(data, &path);
    let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
        .expect("Unable to read input file");
    let parsed_schema = parse_schema(&supergraph_sdl);
    let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
        .expect("Failed to create planner from supergraph");
    let schema_metadata = &planner.consumer_schema.schema_metadata();
    let execution_context = query_plan_executor::QueryPlanExecutionContext {
        variable_values: &None,
        executor: &HTTPSubgraphExecutor::new(HashMap::new()),
        schema_metadata,
        errors: Vec::new(),
        extensions: HashMap::new(),
    };
    let requires_selections: Vec<SelectionItem> =
        [SelectionItem::InlineFragment(InlineFragmentSelection {
            type_condition: "Product".to_string(),
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
            for representation in black_box(&representations) {
                let requires =
                    execution_context.project_requires(&requires_selections, representation);
                black_box(requires);
            }
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
    deep_merge_with_simple(c);
    deep_merge_with_complex(c);
    project_requires(c);
    traverse_and_collect(c);
    project_data_by_operation(c);
    query_plan_executor_without_projection_locally(c);
    query_plan_executor_pipeline_locally(c);

    query_plan_execution_without_projection_via_http(c);
    query_plan_executor_pipeline_via_http(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
