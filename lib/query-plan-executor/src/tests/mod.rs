use std::{collections::HashMap, sync::Arc};

use subgraphs::accounts;

use crate::{executors::async_graphql::AsyncGraphQLExecutor, SubgraphExecutorMap};

#[test]
fn query_executor_pipeline_locally() {
    tokio_test::block_on(async {
        let operation_path = "../../bench/operation.graphql";
        let supergraph_sdl = std::fs::read_to_string("../../bench/supergraph.graphql")
            .expect("Unable to read input file");
        let parsed_schema = query_planner::utils::parsing::parse_schema(&supergraph_sdl);
        let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
            .expect("Failed to create planner from supergraph");
        let parsed_document = query_planner::utils::parsing::parse_operation(
            &std::fs::read_to_string(operation_path).expect("Unable to read input file"),
        );
        let normalized_document = query_planner::ast::normalization::normalize_operation(
            &planner.supergraph,
            &parsed_document,
            None,
        )
        .expect("Failed to normalize operation");
        let normalized_operation = normalized_document.executable_operation();
        let query_plan = planner
            .plan_from_normalized_operation(normalized_operation)
            .expect("Failed to create query plan");
        let schema_metadata =
            crate::schema_metadata::SchemaWithMetadata::schema_metadata(&planner.consumer_schema);
        let mut subgraph_executor_map: SubgraphExecutorMap = HashMap::new(); // No subgraphs in this test
        let accounts = accounts::get_subgraph();
        let inventory = subgraphs::inventory::get_subgraph();
        let products = subgraphs::products::get_subgraph();
        let reviews = subgraphs::reviews::get_subgraph();
        subgraph_executor_map.insert(
            "accounts".to_string(),
            Arc::new(Box::new(AsyncGraphQLExecutor::new(accounts))),
        );
        subgraph_executor_map.insert(
            "inventory".to_string(),
            Arc::new(Box::new(AsyncGraphQLExecutor::new(inventory))),
        );
        subgraph_executor_map.insert(
            "products".to_string(),
            Arc::new(Box::new(AsyncGraphQLExecutor::new(products))),
        );
        subgraph_executor_map.insert(
            "reviews".to_string(),
            Arc::new(Box::new(AsyncGraphQLExecutor::new(reviews))),
        );
        let result = crate::execute_query_plan(
            &query_plan,
            &subgraph_executor_map,
            &None,
            &schema_metadata,
            normalized_operation,
            false,
        )
        .await;
        insta::assert_snapshot!(format!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        ));
    });
}
