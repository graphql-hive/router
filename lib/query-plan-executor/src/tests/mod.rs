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
impl crate::executors::common::SubgraphExecutor for TestExecutor {
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: crate::ExecutionRequest,
    ) -> crate::ExecutionResult {
        match subgraph_name {
            "accounts" => self.accounts.execute(execution_request).await.into(),
            "inventory" => self.inventory.execute(execution_request).await.into(),
            "products" => self.products.execute(execution_request).await.into(),
            "reviews" => self.reviews.execute(execution_request).await.into(),
            _ => crate::ExecutionResult::from_error_message(format!(
                "Subgraph {} not found in schema map",
                subgraph_name
            )),
        }
    }
}

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
        let executor = TestExecutor {
            accounts: subgraphs::accounts::get_subgraph(),
            inventory: subgraphs::inventory::get_subgraph(),
            products: subgraphs::products::get_subgraph(),
            reviews: subgraphs::reviews::get_subgraph(),
        };
        let result = crate::execute_query_plan(
            &query_plan,
            &executor,
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
