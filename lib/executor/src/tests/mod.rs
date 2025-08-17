use query_planner::graph::PlannerOverrideContext;
use sonic_rs::JsonValueTrait;

use crate::response::graphql_error::GraphQLErrorPathSegment;
use crate::{
    context::QueryPlanExecutionContext, execution::plan::QueryPlanExecutor,
    executors::common::SubgraphExecutor, introspection::schema::SchemaWithMetadata,
    SubgraphExecutorMap,
};

mod async_graphql;
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

    let schema_metadata = SchemaWithMetadata::schema_metadata(&planner.consumer_schema);
    let movies_subgraph = fixtures::error_propagation::movies::get_subgraph();
    let directors_subgraph = fixtures::error_propagation::directors::get_subgraph();
    let mut subgraph_executor_map = SubgraphExecutorMap::new();
    subgraph_executor_map.insert_boxed_arc(
        "movies".to_string(),
        SubgraphExecutor::to_boxed_arc(movies_subgraph),
    );
    subgraph_executor_map.insert_boxed_arc(
        "directors".to_string(),
        SubgraphExecutor::to_boxed_arc(directors_subgraph),
    );
    tokio_test::block_on(async {
        let qp_executor = QueryPlanExecutor::new(&None, &subgraph_executor_map, &schema_metadata);
        let mut qp_exec_ctx =
            QueryPlanExecutionContext::new(&query_plan, crate::response::value::Value::Null);
        qp_executor
            .execute(&mut qp_exec_ctx, query_plan.node.as_ref())
            .await;
        assert_eq!(qp_exec_ctx.errors.len(), 1);
        let error = &qp_exec_ctx.errors[0];
        assert_eq!(
            error.path,
            Some(vec![GraphQLErrorPathSegment::String("movie2".to_string())])
        );
        assert_eq!(error.message, "Director not found for movie with id 2");
        assert_eq!(
            error
                .extensions
                .as_ref()
                .map(|ext| ext.get("code").map(|v| v.as_str()))
                .flatten()
                .flatten(),
            Some("DOWNSTREAM_SERVICE_ERROR")
        );
        assert_eq!(
            error
                .extensions
                .as_ref()
                .map(|ext| ext.get("serviceName").map(|v| v.as_str()))
                .flatten()
                .flatten(),
            Some("directors")
        );
        insta::assert_snapshot!(qp_exec_ctx.final_response.to_string());
    });
}
