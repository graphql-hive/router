use query_planner::graph::PlannerOverrideContext;

use subgraphs::accounts;

use crate::{
    executors::{common::SubgraphExecutor, map::SubgraphExecutorMap},
    projection, ErrorsAndExtensions, ExecutableQueryPlan, QueryPlanExecutionContext,
};

mod traverse_and_callback;

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
        let override_context = PlannerOverrideContext::default();
        let query_plan = planner
            .plan_from_normalized_operation(normalized_operation, override_context)
            .expect("Failed to create query plan");
        let schema_metadata =
            crate::schema_metadata::SchemaWithMetadata::schema_metadata(&planner.consumer_schema);
        let mut subgraph_executor_map = SubgraphExecutorMap::new(); // No subgraphs in this test
        let accounts = accounts::get_subgraph();
        let inventory = subgraphs::inventory::get_subgraph();
        let products = subgraphs::products::get_subgraph();
        let reviews = subgraphs::reviews::get_subgraph();
        subgraph_executor_map.insert_boxed_arc("accounts".to_string(), accounts.to_boxed_arc());
        subgraph_executor_map.insert_boxed_arc("inventory".to_string(), inventory.to_boxed_arc());
        subgraph_executor_map.insert_boxed_arc("products".to_string(), products.to_boxed_arc());
        subgraph_executor_map.insert_boxed_arc("reviews".to_string(), reviews.to_boxed_arc());
        let (root_type_name, projection_selections) =
            projection::FieldProjectionPlan::from_operation(normalized_operation, &schema_metadata);
        let result = crate::execute_query_plan(
            &query_plan,
            &subgraph_executor_map,
            &None,
            &schema_metadata,
            root_type_name,
            &projection_selections,
            false,
            crate::ExposeQueryPlanMode::No,
        )
        .await;
        insta::assert_snapshot!(String::from_utf8(result.unwrap().to_vec()).unwrap(),);
    });
}

mod fixtures;

#[test]
fn error_propagation() {
    let supergraph_sdl =
        std::fs::read_to_string("./src/tests/fixtures/error_propagation/supergraph.graphql")
            .expect("Unable to read input file");
    let parsed_schema = query_planner::utils::parsing::parse_schema(&supergraph_sdl);
    let planner = query_planner::planner::Planner::new_from_supergraph(&parsed_schema)
        .expect("Failed to create planner from supergraph");
    let parsed_document = query_planner::utils::parsing::parse_operation(
        &std::fs::read_to_string("./src/tests/fixtures/error_propagation/operation.graphql")
            .expect("Unable to read input file"),
    );
    let normalized_document = query_planner::ast::normalization::normalize_operation(
        &planner.supergraph,
        &parsed_document,
        None,
    )
    .expect("Failed to normalize operation");
    let normalized_operation = normalized_document.executable_operation();
    let query_plan = planner
        .plan_from_normalized_operation(normalized_operation, PlannerOverrideContext::default())
        .expect("Failed to create query plan");

    let schema_metadata =
        crate::schema_metadata::SchemaWithMetadata::schema_metadata(&planner.consumer_schema);
    let movies_subgraph = fixtures::error_propagation::movies::get_subgraph();
    let directors_subgraph = fixtures::error_propagation::directors::get_subgraph();
    let mut subgraph_executor_map = SubgraphExecutorMap::new();
    subgraph_executor_map.insert_boxed_arc("movies".to_string(), movies_subgraph.to_boxed_arc());
    subgraph_executor_map
        .insert_boxed_arc("directors".to_string(), directors_subgraph.to_boxed_arc());
    tokio_test::block_on(async {
        let mut result_data = serde_json::json!({});
        let execution_context = QueryPlanExecutionContext {
            variable_values: &None,
            subgraph_executor_map: &subgraph_executor_map,
            schema_metadata: &schema_metadata,
        };
        let mut errors_and_extensions = ErrorsAndExtensions::default();
        query_plan
            .execute(
                &execution_context,
                &mut result_data,
                &mut errors_and_extensions,
            )
            .await;
        insta::assert_snapshot!(result_data);
        assert_eq!(errors_and_extensions.errors.len(), 1);
        let error = &errors_and_extensions.errors[0];
        assert_eq!(error.message, "Director not found for movie with id 2");
        assert_eq!(
            error.extensions.as_ref().unwrap().get("code"),
            Some(&serde_json::Value::String(
                "DOWNSTREAM_SERVICE_ERROR".to_string()
            ))
        );
        assert_eq!(
            error.extensions.as_ref().unwrap().get("serviceName"),
            Some(&serde_json::Value::String("directors".to_string()))
        );
        assert_eq!(
            error.path,
            Some(vec![serde_json::Value::String("movie2".to_string())])
        );
    });
}
