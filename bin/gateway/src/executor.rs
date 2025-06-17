use std::sync::Arc;

#[derive(Clone)]
pub struct GWExecutor {
    pub app_state: Arc<crate::AppState>,
}

impl async_graphql::Executor for GWExecutor {
    async fn execute(&self, execution_request: async_graphql::Request) -> async_graphql::Response {
        let original_document =
            match query_planner::utils::parsing::safe_parse_operation(&execution_request.query) {
                Ok(doc) => doc,
                Err(err) => {
                    return async_graphql::Response::from_errors(vec![
                        async_graphql::ServerError::new(err.to_string(), None),
                    ]);
                }
            };

        tracing::debug!(original_document = %original_document, "Original document parsed");

        let normalized_document = match query_planner::ast::normalization::normalize_operation(
            &self.app_state.planner.supergraph,
            &original_document,
            execution_request.operation_name.as_deref(),
        ) {
            Ok(doc) => doc,
            Err(err) => {
                tracing::error!("Normalization error {err}");

                return async_graphql::Response::from_errors(vec![
                    async_graphql::ServerError::new(
                        "Unable to detect operation AST".to_string(),
                        None,
                    ),
                ]);
            }
        };
        tracing::debug!(normalized_document = %normalized_document, "Normalized document prepared");

        let operation = normalized_document.operation;
        tracing::debug!(executable_operation = %operation, "Executable operation obtained");

        let (has_introspection, filtered_operation_for_plan) =
            query_plan_executor::introspection::filter_introspection_fields_in_operation(
                &operation,
            );

        // GraphQL Over HTTP specification requires us to return 400
        // (in case of Accept: application/graphql-response+json)
        // on variable value coerion failures.
        // That's why collection of variables is happening before validations.
        let variable_values = match query_plan_executor::variables::collect_variables(
            &filtered_operation_for_plan,
            &execution_request.variables,
            &self.app_state.schema_metadata,
        ) {
            Ok(values) => values,
            Err(err_msg) => {
                return async_graphql::Response::from_errors(vec![
                    async_graphql::ServerError::new(err_msg, None),
                ]);
            }
        };
        tracing::debug!(variables = ?variable_values, "Variables collected");

        let consumer_schema_ast = &self.app_state.planner.consumer_schema.document;
        let validation_cache_key = operation.hash();
        let validation_result = match self
            .app_state
            .validate_cache
            .get(&validation_cache_key)
            .await
        {
            Some(cached_validation) => cached_validation,
            None => {
                let res = graphql_tools::validation::validate::validate(
                    consumer_schema_ast,
                    &original_document,
                    &self.app_state.validation_plan,
                );
                let arc_res = Arc::new(res);
                self.app_state
                    .validate_cache
                    .insert(validation_cache_key, arc_res.clone())
                    .await;
                arc_res
            }
        };

        if !validation_result.is_empty() {
            tracing::debug!(validation_errors = ?validation_result, "Validation failed");
            let errors = validation_result
                .iter()
                .map(query_plan_executor::validation::from_validation_error_to_server_error)
                .collect::<Vec<_>>();
            let error_result = async_graphql::Response::from_errors(errors);
            return error_result;
        }
        tracing::debug!("Validation successful");

        let plan_cache_key = filtered_operation_for_plan.hash();

        let query_plan_arc = match self.app_state.plan_cache.get(&plan_cache_key).await {
            Some(plan) => plan,
            None => {
                let plan =
                    if filtered_operation_for_plan.selection_set.is_empty() && has_introspection {
                        query_planner::planner::plan_nodes::QueryPlan {
                            kind: "QueryPlan".to_string(),
                            node: None,
                        }
                    } else {
                        match self
                            .app_state
                            .planner
                            .plan_from_normalized_operation(&filtered_operation_for_plan)
                        {
                            Ok(p) => p,
                            Err(err) => {
                                return async_graphql::Response::from_errors(vec![
                                    async_graphql::ServerError::new(err.to_string(), None),
                                ]);
                            }
                        }
                    };
                let arc_plan = Arc::new(plan);
                self.app_state
                    .plan_cache
                    .insert(plan_cache_key, arc_plan.clone())
                    .await;
                arc_plan
            }
        };
        tracing::debug!(query_plan = ?query_plan_arc, "Query plan obtained/generated");

        let execution_result = query_plan_executor::execute_query_plan(
            &query_plan_arc,
            &self.app_state.subgraph_executor_map,
            &variable_values,
            &self.app_state.schema_metadata,
            &operation,
            has_introspection,
        )
        .await;

        tracing::debug!(execution_result = ?execution_result, "Execution result");

        execution_result
    }

    fn execute_stream(
        &self,
        _request: async_graphql::Request,
        _session_data: Option<Arc<async_graphql::Data>>,
    ) -> futures::stream::BoxStream<'static, async_graphql::Response> {
        unimplemented!("HTTPSubgraphExecutor does not support streaming execution yet.")
    }
}
