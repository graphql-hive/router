#![recursion_limit = "256"]
use std::collections::HashMap;

use criterion::black_box;
use criterion::Criterion;
use criterion::{criterion_group, criterion_main};

use query_plan_executor::execute_query_plan;
use query_plan_executor::schema_metadata::SchemaWithMetadata;
use query_plan_executor::ExecutableQueryPlan;
use query_plan_executor::HTTPSubgraphExecutor;
use query_planner::ast::normalization::normalize_operation;
use query_planner::utils::parsing::parse_operation;
use query_planner::utils::parsing::parse_schema;
use serde_json::json;
use serde_json::Value;
// This is a struct that tells Criterion.rs to use the "futures" crate's current-thread executor
use tokio::runtime::Runtime;

fn query_plan_executor_pipeline(c: &mut Criterion) {
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
    c.bench_function("query_plan_executor_pipeline", |b| {
        b.to_async(&rt).iter(|| async {
            let query_plan = black_box(&query_plan);
            let subgraph_endpoint_map = black_box(&subgraph_endpoint_map);
            let schema_metadata = black_box(&schema_metadata);
            let operation = black_box(&normalized_operation);
            let has_introspection = false;
            let http_client = black_box(&http_client);
            let result = execute_query_plan(
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

fn query_plan_execution_without_projection(c: &mut Criterion) {
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
    c.bench_function("query_plan_execution_without_projection", |b| {
        b.to_async(&rt).iter(|| async {
            let schema_metadata = black_box(&schema_metadata);
            let subgraph_endpoint_map = black_box(&subgraph_endpoint_map);
            let http_client = black_box(&http_client);
            let mut execution_context = query_plan_executor::QueryPlanExecutionContext {
                variable_values: &None,
                schema_metadata: schema_metadata,
                executor: HTTPSubgraphExecutor {
                    subgraph_endpoint_map: subgraph_endpoint_map,
                    http_client: http_client,
                },
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
            let mut data = json!({
                "data": {
                  "users": [
                    {
                      "__typename": "User",
                      "id": "1",
                      "name": "Uri Goldshtein",
                      "username": "urigo",
                      "reviews": [
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          "id": "1"
                        },
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          "id": "2"
                        }
                      ]
                    },
                    {
                      "__typename": "User",
                      "id": "2",
                      "name": "Dotan Simha",
                      "username": "dotansimha",
                      "reviews": [
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          "id": "1"
                        },
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          "id": "2"
                        }
                      ]
                    },
                    {
                      "__typename": "User",
                      "id": "3",
                      "name": "Kamil Kisiela",
                      "username": "kamilkisiela",
                      "reviews": [
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          "id": "1"
                        },
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          "id": "2"
                        }
                      ]
                    },
                    {
                      "__typename": "User",
                      "id": "4",
                      "name": "Arda Tanrikulu",
                      "username": "ardatan",
                      "reviews": [
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          "id": "1"
                        },
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          "id": "2"
                        }
                      ]
                    },
                    {
                      "__typename": "User",
                      "id": "5",
                      "name": "Gil Gardosh",
                      "username": "gilgardosh",
                      "reviews": [
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          "id": "1"
                        },
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          "id": "2"
                        }
                      ]
                    },
                    {
                      "__typename": "User",
                      "id": "6",
                      "name": "Laurin Quast",
                      "username": "laurin",
                      "reviews": [
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          "id": "1"
                        },
                        {
                          "product": {
                            "__typename": "Product",
                            "reviews": [
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                                "id": "3"
                              },
                              {
                                "author": {
                                  "__typename": "User",
                                  "reviews": [
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                      "id": "1"
                                    },
                                    {
                                      "product": {
                                        "__typename": "Product",
                                        "upc": "1",
                                        "price": 899,
                                        "weight": 100,
                                        "name": "Table",
                                        "inStock": true,
                                        "shippingEstimate": null
                                      },
                                      "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                      "id": "2"
                                    }
                                  ],
                                  "id": "1",
                                  "username": "urigo",
                                  "name": "Uri Goldshtein"
                                },
                                "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                                "id": "4"
                              }
                            ],
                            "upc": "1",
                            "price": 899,
                            "weight": 100,
                            "name": "Table",
                            "inStock": true,
                            "shippingEstimate": null
                          },
                          "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          "id": "2"
                        }
                      ]
                    }
                  ],
                  "topProducts": [
                    {
                      "__typename": "Product",
                      "upc": "1",
                      "weight": 100,
                      "price": 899,
                      "name": "Table",
                      "shippingEstimate": null,
                      "inStock": true,
                      "reviews": [
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          "id": "1"
                        },
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          "id": "2"
                        },
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                          "id": "3"
                        },
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                          "id": "4"
                        }
                      ]
                    },
                    {
                      "__typename": "Product",
                      "upc": "2",
                      "weight": 1000,
                      "price": 1299,
                      "name": "Couch",
                      "shippingEstimate": null,
                      "inStock": false,
                      "reviews": [
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          "id": "5"
                        },
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          "id": "6"
                        },
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                          "id": "7"
                        },
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                          "id": "8"
                        }
                      ]
                    },
                    {
                      "__typename": "Product",
                      "upc": "3",
                      "weight": 20,
                      "price": 15,
                      "name": "Glass",
                      "shippingEstimate": null,
                      "inStock": false,
                      "reviews": [
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                          "id": "9"
                        }
                      ]
                    },
                    {
                      "__typename": "Product",
                      "upc": "4",
                      "weight": 100,
                      "price": 499,
                      "name": "Chair",
                      "shippingEstimate": null,
                      "inStock": false,
                      "reviews": [
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                          "id": "10"
                        },
                        {
                          "author": {
                            "__typename": "User",
                            "reviews": [
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                                "id": "1"
                              },
                              {
                                "product": {
                                  "__typename": "Product",
                                  "upc": "1",
                                  "price": 899,
                                  "weight": 100,
                                  "name": "Table",
                                  "inStock": true,
                                  "shippingEstimate": null
                                },
                                "body": "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                                "id": "2"
                              }
                            ],
                            "id": "1",
                            "username": "urigo",
                            "name": "Uri Goldshtein"
                          },
                          "body": "At vero eos et accusamus et iusto odio dignissimos ducimus qui blanditiis praesentium voluptatum deleniti atque corrupti quos dolores et quas molestias excepturi sint occaecati cupiditate non provident, similique sunt in culpa qui officia deserunt mollitia animi, id est laborum et dolorum fuga. Et harum quidem rerum facilis est et expedita distinctio. Nam libero tempore, cum soluta nobis est eligendi optio cumque nihil impedit quo minus id quod maxime placeat facere possimus, omnis voluptas assumenda est, omnis dolor repellendus. Temporibus autem quibusdam et aut officiis debitis aut rerum necessitatibus saepe eveniet ut et voluptates repudiandae sint et molestiae non recusandae. Itaque earum rerum hic tenetur a sapiente delectus, ut aut reiciendis voluptatibus maiores alias consequatur aut perferendis doloribus asperiores repellat.",
                          "id": "11"
                        }
                      ]
                    },
                    {
                      "__typename": "Product",
                      "upc": "5",
                      "weight": 1000,
                      "price": 1299,
                      "name": "TV",
                      "shippingEstimate": null,
                      "inStock": true,
                      "reviews": []
                    }
                  ]
                },
                "extensions": {
                  "queryPlan": {
                    "kind": "QueryPlan",
                    "node": {
                      "kind": "Sequence",
                      "nodes": [
                        {
                          "kind": "Parallel",
                          "nodes": [
                            {
                              "kind": "Fetch",
                              "serviceName": "accounts",
                              "operationKind": "query",
                              "operation": "{users{__typename id name username}}"
                            },
                            {
                              "kind": "Fetch",
                              "serviceName": "products",
                              "operationKind": "query",
                              "operation": "{topProducts{__typename upc weight price name}}"
                            }
                          ]
                        },
                        {
                          "kind": "Parallel",
                          "nodes": [
                            {
                              "kind": "Flatten",
                              "path": [
                                "users",
                                "@"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "reviews",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{reviews{product{__typename reviews{author{__typename reviews{product{__typename upc} body id} id username} body id} upc} body id}}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "User",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "id"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "topProducts",
                                "@"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "inventory",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate inStock}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "price"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "weight"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "topProducts",
                                "@"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "reviews",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{reviews{author{__typename reviews{product{__typename upc} body id} id username} body id}}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            }
                          ]
                        },
                        {
                          "kind": "Parallel",
                          "nodes": [
                            {
                              "kind": "Flatten",
                              "path": [
                                "users",
                                "@",
                                "reviews",
                                "@",
                                "product",
                                "reviews",
                                "@",
                                "author",
                                "reviews",
                                "@",
                                "product"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "products",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price weight name}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "users",
                                "@",
                                "reviews",
                                "@",
                                "product"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "products",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price weight name}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "users",
                                "@",
                                "reviews",
                                "@",
                                "product"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "inventory",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "users",
                                "@",
                                "reviews",
                                "@",
                                "product",
                                "reviews",
                                "@",
                                "author"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "accounts",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "User",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "id"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "users",
                                "@",
                                "reviews",
                                "@",
                                "product",
                                "reviews",
                                "@",
                                "author",
                                "reviews",
                                "@",
                                "product"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "inventory",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "topProducts",
                                "@",
                                "reviews",
                                "@",
                                "author",
                                "reviews",
                                "@",
                                "product"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "products",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{price weight name}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "topProducts",
                                "@",
                                "reviews",
                                "@",
                                "author"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "accounts",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on User{name}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "User",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "id"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "topProducts",
                                "@",
                                "reviews",
                                "@",
                                "author",
                                "reviews",
                                "@",
                                "product"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "inventory",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{inStock}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            }
                          ]
                        },
                        {
                          "kind": "Parallel",
                          "nodes": [
                            {
                              "kind": "Flatten",
                              "path": [
                                "users",
                                "@",
                                "reviews",
                                "@",
                                "product",
                                "reviews",
                                "@",
                                "author",
                                "reviews",
                                "@",
                                "product"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "inventory",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "price"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "weight"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "users",
                                "@",
                                "reviews",
                                "@",
                                "product"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "inventory",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "price"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "weight"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            },
                            {
                              "kind": "Flatten",
                              "path": [
                                "topProducts",
                                "@",
                                "reviews",
                                "@",
                                "author",
                                "reviews",
                                "@",
                                "product"
                              ],
                              "node": {
                                "kind": "Fetch",
                                "serviceName": "inventory",
                                "operationKind": "query",
                                "operation": "query($representations:[_Any!]!){_entities(representations: $representations){...on Product{shippingEstimate}}}",
                                "requires": [
                                  {
                                    "kind": "InlineFragment",
                                    "typeCondition": "Product",
                                    "selections": [
                                      {
                                        "kind": "Field",
                                        "name": "__typename"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "price"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "weight"
                                      },
                                      {
                                        "kind": "Field",
                                        "name": "upc"
                                      }
                                    ]
                                  }
                                ]
                              }
                            }
                          ]
                        }
                      ]
                    }
                  }
                }
              });
            let data = black_box(&mut data);
            let operation = black_box(&operation);
            let schema_metadata = black_box(&schema_metadata);
            let result = query_plan_executor::project_data_by_operation(
                data,
                &mut vec![],
                operation,
                schema_metadata,
                &None,
            );
            black_box(result);
        });
    });
}

fn all_benchmarks(c: &mut Criterion) {
    query_plan_execution_without_projection(c);
    project_data_by_operation(c);
    query_plan_executor_pipeline(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
