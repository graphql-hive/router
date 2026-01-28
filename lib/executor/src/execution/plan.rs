use std::collections::{BTreeSet, HashMap};

use bytes::{BufMut, Bytes};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use hive_router_query_planner::planner::plan_nodes::{
    ConditionNode, FetchNode, FetchRewrite, FlattenNode, FlattenNodePath, PlanNode, QueryPlan,
};
use http::HeaderMap;
use sonic_rs::ValueRef;

use crate::{
    context::ExecutionContext,
    execution::{
        client_request_details::ClientRequestDetails,
        error::{IntoPlanExecutionError, LazyPlanContext, PlanExecutionError},
        jwt_forward::JwtAuthForwardingPlan,
        rewrites::FetchRewriteExt,
    },
    executors::{common::SubgraphExecutionRequest, map::SubgraphExecutorMap},
    headers::{
        plan::HeaderRulesPlan,
        request::modify_subgraph_request_headers,
        response::{apply_subgraph_response_headers, modify_client_response_headers},
    },
    introspection::{
        resolve::{resolve_introspection, IntrospectionContext},
        schema::SchemaMetadata,
    },
    projection::{
        plan::FieldProjectionPlan,
        request::{project_requires, RequestProjectionContext},
        response::project_by_operation,
    },
    response::{
        graphql_error::{GraphQLError, GraphQLErrorPath},
        merge::deep_merge,
        subgraph_response::SubgraphResponse,
        value::Value,
    },
    utils::{
        consts::{CLOSE_BRACKET, OPEN_BRACKET},
        traverse::{traverse_and_callback, traverse_and_callback_mut},
    },
};

pub struct QueryPlanExecutionContext<'exec> {
    pub query_plan: &'exec QueryPlan,
    pub projection_plan: &'exec [FieldProjectionPlan],
    pub headers_plan: &'exec HeaderRulesPlan,
    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    pub extensions: Option<HashMap<String, sonic_rs::Value>>,
    pub client_request: &'exec ClientRequestDetails<'exec>,
    pub introspection_context: &'exec IntrospectionContext<'exec, 'static>,
    pub operation_type_name: &'exec str,
    pub executors: &'exec SubgraphExecutorMap,
    pub jwt_auth_forwarding: &'exec Option<JwtAuthForwardingPlan>,
    pub initial_errors: Vec<GraphQLError>,
}

pub struct PlanExecutionOutput {
    pub body: Vec<u8>,
    pub headers: HeaderMap,
    pub error_count: usize,
}

pub async fn execute_query_plan<'exec>(
    ctx: QueryPlanExecutionContext<'exec>,
) -> Result<PlanExecutionOutput, PlanExecutionError> {
    let init_value = if let Some(introspection_query) = ctx.introspection_context.query {
        resolve_introspection(introspection_query, ctx.introspection_context)
    } else if ctx.projection_plan.is_empty() {
        Value::Null
    } else {
        Value::Object(Vec::new())
    };

    let mut exec_ctx = ExecutionContext::new(ctx.query_plan, init_value, ctx.initial_errors);
    let executor = Executor::new(
        ctx.variable_values,
        ctx.executors,
        ctx.introspection_context.metadata,
        ctx.client_request,
        ctx.headers_plan,
        ctx.jwt_auth_forwarding,
        // Deduplicate subgraph requests only if the operation type is a query
        ctx.operation_type_name == "Query",
    );

    if let Some(node) = &ctx.query_plan.node {
        executor.execute_plan_node(&mut exec_ctx, node).await?;
    }

    let mut response_headers = HeaderMap::new();
    modify_client_response_headers(exec_ctx.response_headers_aggregator, &mut response_headers)
        .with_plan_context(LazyPlanContext {
            subgraph_name: || None,
            affected_path: || None,
        })?;

    let final_response = &exec_ctx.final_response;
    let error_count = exec_ctx.errors.len(); // Added for usage reporting
    let body = project_by_operation(
        final_response,
        exec_ctx.errors,
        &ctx.extensions,
        ctx.operation_type_name,
        ctx.projection_plan,
        ctx.variable_values,
        exec_ctx.response_storage.estimate_final_response_size(),
        ctx.introspection_context.metadata,
    )
    .with_plan_context(LazyPlanContext {
        subgraph_name: || None,
        affected_path: || None,
    })?;

    Ok(PlanExecutionOutput {
        body,
        headers: response_headers,
        error_count,
    })
}

pub struct Executor<'exec> {
    variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    schema_metadata: &'exec SchemaMetadata,
    executors: &'exec SubgraphExecutorMap,
    client_request: &'exec ClientRequestDetails<'exec>,
    headers_plan: &'exec HeaderRulesPlan,
    jwt_forwarding_plan: &'exec Option<JwtAuthForwardingPlan>,
    dedupe_subgraph_requests: bool,
}

struct ConcurrencyScope<'exec, T> {
    jobs: FuturesUnordered<BoxFuture<'exec, T>>,
}

impl<'exec, T> ConcurrencyScope<'exec, T> {
    fn new() -> Self {
        Self {
            jobs: FuturesUnordered::new(),
        }
    }

    fn spawn(&mut self, future: BoxFuture<'exec, T>) {
        self.jobs.push(future);
    }

    async fn join_all(mut self) -> Vec<T> {
        let mut results = Vec::with_capacity(self.jobs.len());
        while let Some(result) = self.jobs.next().await {
            results.push(result);
        }
        results
    }
}

struct FetchJob<'exec> {
    fetch_node_id: i64,
    subgraph_name: &'exec str,
    response: SubgraphResponse<'exec>,
}

struct FlattenFetchJob<'exec> {
    flatten_node_path: &'exec FlattenNodePath,
    response: SubgraphResponse<'exec>,
    fetch_node_id: i64,
    subgraph_name: &'exec str,
    representation_hashes: Vec<u64>,
    representation_hash_to_index: HashMap<u64, usize>,
}

enum ExecutionJob<'exec> {
    Fetch(FetchJob<'exec>),
    FlattenFetch(FlattenFetchJob<'exec>),
}

impl<'exec> Executor<'exec> {
    pub fn new(
        variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
        executors: &'exec SubgraphExecutorMap,
        schema_metadata: &'exec SchemaMetadata,
        client_request: &'exec ClientRequestDetails<'exec>,
        headers_plan: &'exec HeaderRulesPlan,
        jwt_forwarding_plan: &'exec Option<JwtAuthForwardingPlan>,
        dedupe_subgraph_requests: bool,
    ) -> Self {
        Executor {
            variable_values,
            executors,
            schema_metadata,
            client_request,
            headers_plan,
            dedupe_subgraph_requests,
            jwt_forwarding_plan,
        }
    }

    pub async fn execute_plan_node(
        &'exec self,
        ctx: &mut ExecutionContext<'exec>,
        plan: &'exec PlanNode,
    ) -> Result<(), PlanExecutionError> {
        match plan {
            PlanNode::Fetch(node) => match self.execute_fetch_node(node, None).await {
                Ok(result) => self.process_job_result(ctx, result),
                Err(err) => {
                    self.log_error(&err);
                    ctx.errors.push(err.into());
                    Ok(())
                }
            },
            PlanNode::Parallel(node) => {
                let mut scope = ConcurrencyScope::new();

                for child in &node.nodes {
                    let job_future = self.prepare_job_future(child, &ctx.final_response);
                    scope.spawn(job_future);
                }

                let results = scope.join_all().await;

                for result in results {
                    match result {
                        Ok(Some(job)) => {
                            self.process_job_result(ctx, job)?;
                        }
                        Ok(None) => { /* do nothing */ }
                        Err(err) => {
                            self.log_error(&err);
                            ctx.errors.push(err.into())
                        }
                    }
                }

                Ok(())
            }
            PlanNode::Sequence(node) => {
                for child in &node.nodes {
                    // Box::pin the future for recursive calls to have the correct lifetime
                    // self.execute_plan_node can call back into execute_sequence_wave
                    Box::pin(self.execute_plan_node(ctx, child)).await?;
                }

                Ok(())
            }
            PlanNode::Flatten(node) => {
                let Some(job) = self
                    .execute_flatten_fetch_node(node, &ctx.final_response)
                    .await?
                else {
                    return Ok(());
                };

                self.process_job_result(ctx, job)
            }
            PlanNode::Condition(node) => {
                let Some(node) = condition_node_by_variables(node, self.variable_values) else {
                    return Ok(());
                };
                // Box::pin the future for recursive calls to have the correct lifetime
                Box::pin(self.execute_plan_node(ctx, node)).await
            }
            // Plans produced by our Query Planner can only start with: Fetch, Sequence or Parallel.
            // Any other node type at the root is not supported, do nothing
            _ => Ok(()),
        }
    }

    fn prepare_job_future<'wave>(
        &'exec self,
        node: &'exec PlanNode,
        final_response: &'wave Value<'exec>,
    ) -> BoxFuture<'wave, Result<Option<ExecutionJob<'exec>>, PlanExecutionError>> {
        Box::pin(async move {
            match node {
                PlanNode::Fetch(fetch_node) => {
                    Ok(Some(self.execute_fetch_node(fetch_node, None).await?))
                }
                PlanNode::Flatten(flatten_node) => Ok(self
                    .execute_flatten_fetch_node(flatten_node, final_response)
                    .await?),
                PlanNode::Condition(node) => {
                    let Some(node) = condition_node_by_variables(node, self.variable_values) else {
                        return Ok(None);
                    };
                    self.prepare_job_future(node, final_response).await
                }
                // Our Query Planner does not produce any other plan node types in ParallelNode
                _ => Ok(None),
            }
        })
    }

    fn process_subgraph_response(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        response_bytes: Option<Bytes>,
        fetch_node_id: i64,
    ) -> Option<&'exec [FetchRewrite]> {
        if let Some(response_bytes) = response_bytes {
            ctx.response_storage.add_response(response_bytes);
        }

        ctx.output_rewrites.get(fetch_node_id)
    }

    fn process_job_result(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        job: ExecutionJob<'exec>,
    ) -> Result<(), PlanExecutionError> {
        match job {
            ExecutionJob::Fetch(mut job) => {
                if let Some(response_headers) = &job.response.headers {
                    apply_subgraph_response_headers(
                        self.headers_plan,
                        job.subgraph_name,
                        response_headers,
                        self.client_request,
                        &mut ctx.response_headers_aggregator,
                    )
                    .with_plan_context(LazyPlanContext {
                        subgraph_name: || Some(job.subgraph_name.into()),
                        affected_path: || None,
                    })?;
                }

                if let Some(output_rewrites) =
                    self.process_subgraph_response(ctx, job.response.bytes, job.fetch_node_id)
                {
                    for output_rewrite in output_rewrites {
                        output_rewrite
                            .rewrite(&self.schema_metadata.possible_types, &mut job.response.data);
                    }
                }

                ctx.handle_errors(job.subgraph_name, None, job.response.errors, None);

                deep_merge(&mut ctx.final_response, job.response.data);
            }
            ExecutionJob::FlattenFetch(mut job) => {
                if let Some(response_headers) = &job.response.headers {
                    apply_subgraph_response_headers(
                        self.headers_plan,
                        job.subgraph_name,
                        response_headers,
                        self.client_request,
                        &mut ctx.response_headers_aggregator,
                    )
                    .with_plan_context(LazyPlanContext {
                        subgraph_name: || Some(job.subgraph_name.into()),
                        affected_path: || None,
                    })?;
                }

                let output_rewrites =
                    self.process_subgraph_response(ctx, job.response.bytes, job.fetch_node_id);

                let mut entity_index_error_map: Option<HashMap<&usize, Vec<GraphQLErrorPath>>> =
                    None;

                if let Some(mut entities) = job.response.data.take_entities() {
                    if let Some(output_rewrites) = output_rewrites {
                        for output_rewrite in output_rewrites {
                            for entity in &mut entities {
                                output_rewrite
                                    .rewrite(&self.schema_metadata.possible_types, entity);
                            }
                        }
                    }

                    let mut index = 0;
                    let normalized_path = job.flatten_node_path.as_slice();
                    // If there is an error in the response, then collect the paths for normalizing the error
                    let initial_error_path = job
                        .response
                        .errors
                        .as_ref()
                        .map(|_| GraphQLErrorPath::with_capacity(normalized_path.len() + 2));
                    entity_index_error_map = job
                        .response
                        .errors
                        .as_ref()
                        .map(|_| HashMap::with_capacity(entities.len()));
                    traverse_and_callback_mut(
                        &mut ctx.final_response,
                        normalized_path,
                        self.schema_metadata,
                        initial_error_path,
                        &mut |target, error_path| {
                            let hash = job.representation_hashes[index];
                            if let Some(entity_index) = job.representation_hash_to_index.get(&hash)
                            {
                                if let (Some(error_path), Some(entity_index_error_map)) =
                                    (error_path, entity_index_error_map.as_mut())
                                {
                                    let error_paths = entity_index_error_map
                                        .entry(entity_index)
                                        .or_insert_with(Vec::new);
                                    error_paths.push(error_path);
                                }
                                if let Some(entity) = entities.get(*entity_index) {
                                    deep_merge(target, entity.clone());
                                }
                            }
                            index += 1;
                        },
                    );
                }
                ctx.handle_errors(
                    job.subgraph_name,
                    Some(job.flatten_node_path),
                    job.response.errors,
                    entity_index_error_map,
                );
            }
        }
        Ok(())
    }

    async fn execute_flatten_fetch_node<'wave>(
        &'exec self,
        flatten_node: &'exec FlattenNode,
        final_response: &'wave Value<'exec>,
    ) -> Result<Option<ExecutionJob<'exec>>, PlanExecutionError> {
        let PlanNode::Fetch(fetch_node) = flatten_node.node.as_ref() else {
            return Ok(None);
        };

        let requires_nodes = match fetch_node.requires.as_ref() {
            Some(nodes) => nodes,
            None => return Ok(None),
        };

        let mut index = 0;
        let normalized_path = flatten_node.path.as_slice();
        let mut filtered_representations = Vec::new();
        filtered_representations.put(OPEN_BRACKET);
        let proj_ctx = RequestProjectionContext::new(&self.schema_metadata.possible_types);
        let mut representation_hashes: Vec<u64> = Vec::new();
        let mut filtered_representations_hashes: HashMap<u64, usize> = HashMap::new();
        let arena = bumpalo::Bump::new();

        traverse_and_callback(
            final_response,
            normalized_path,
            self.schema_metadata,
            &mut |entity| {
                let hash = entity.to_hash(&requires_nodes.items, proj_ctx.possible_types);

                if !entity.is_null() {
                    representation_hashes.push(hash);
                }

                if filtered_representations_hashes.contains_key(&hash) {
                    return Ok::<(), PlanExecutionError>(());
                }

                let entity = if let Some(input_rewrites) = &fetch_node.input_rewrites {
                    let new_entity = arena.alloc(entity.clone());
                    for input_rewrite in input_rewrites {
                        input_rewrite.rewrite(&self.schema_metadata.possible_types, new_entity);
                    }
                    new_entity
                } else {
                    entity
                };

                let is_projected = project_requires(
                    &proj_ctx,
                    &requires_nodes.items,
                    entity,
                    &mut filtered_representations,
                    filtered_representations_hashes.is_empty(),
                    None,
                )
                .with_plan_context(LazyPlanContext {
                    subgraph_name: || Some(fetch_node.service_name.clone()),
                    affected_path: || Some(flatten_node.path.to_string()),
                })?;

                if is_projected {
                    filtered_representations_hashes.insert(hash, index);
                }

                index += 1;

                Ok(())
            },
        )?;
        filtered_representations.put(CLOSE_BRACKET);

        if filtered_representations_hashes.is_empty() {
            return Ok(None);
        }

        let ExecutionJob::Fetch(job) = self
            .execute_fetch_node(fetch_node, Some(filtered_representations))
            .await?
        else {
            return Ok(None);
        };

        Ok(Some(ExecutionJob::FlattenFetch(FlattenFetchJob {
            flatten_node_path: &flatten_node.path,
            response: job.response,
            fetch_node_id: fetch_node.id,
            subgraph_name: &fetch_node.service_name,
            representation_hashes,
            representation_hash_to_index: filtered_representations_hashes,
        })))
    }

    async fn execute_fetch_node(
        &'exec self,
        node: &'exec FetchNode,
        representations: Option<Vec<u8>>,
    ) -> Result<ExecutionJob<'exec>, PlanExecutionError> {
        // TODO: We could optimize header map creation by caching them per service name
        let mut headers_map = HeaderMap::new();
        modify_subgraph_request_headers(
            self.headers_plan,
            &node.service_name,
            self.client_request,
            &mut headers_map,
        )
        .with_plan_context(LazyPlanContext {
            subgraph_name: || Some(node.service_name.clone()),
            affected_path: || None,
        })?;
        let variable_refs =
            select_fetch_variables(self.variable_values, node.variable_usages.as_ref());

        let mut subgraph_request = SubgraphExecutionRequest {
            query: node.operation.document_str.as_str(),
            dedupe: self.dedupe_subgraph_requests,
            operation_name: node.operation_name.as_deref(),
            variables: variable_refs,
            representations,
            headers: headers_map,
            extensions: None,
        };

        if let Some(jwt_forwarding_plan) = &self.jwt_forwarding_plan {
            subgraph_request.add_request_extensions_field(
                jwt_forwarding_plan.extension_field_name.clone(),
                jwt_forwarding_plan.extension_field_value.clone(),
            );
        }

        Ok(ExecutionJob::Fetch(FetchJob {
            fetch_node_id: node.id,
            subgraph_name: &node.service_name,
            response: self
                .executors
                .execute(&node.service_name, subgraph_request, self.client_request)
                .await,
        }))
    }

    fn log_error(&self, error: &PlanExecutionError) {
        tracing::error!(
            subgraph_name = error.subgraph_name(),
            error = error as &dyn std::error::Error,
            "Plan execution error"
        );
    }
}

fn condition_node_by_variables<'a>(
    condition_node: &'a ConditionNode,
    variable_values: &'a Option<HashMap<String, sonic_rs::Value>>,
) -> Option<&'a PlanNode> {
    let vars = variable_values.as_ref()?;
    let value = vars.get(&condition_node.condition)?;
    let condition_met = matches!(value.as_ref(), ValueRef::Bool(true));

    if condition_met {
        condition_node.if_clause.as_deref()
    } else {
        condition_node.else_clause.as_deref()
    }
}

fn select_fetch_variables<'a>(
    variable_values: &'a Option<HashMap<String, sonic_rs::Value>>,
    variable_usages: Option<&BTreeSet<String>>,
) -> Option<HashMap<&'a str, &'a sonic_rs::Value>> {
    let values = variable_values.as_ref()?;

    variable_usages.map(|variable_usages| {
        variable_usages
            .iter()
            .filter_map(|var_name| {
                values
                    .get_key_value(var_name.as_str())
                    .map(|(key, value)| (key.as_str(), value))
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        context::ExecutionContext,
        response::graphql_error::{GraphQLErrorExtensions, GraphQLErrorPath},
    };

    use super::select_fetch_variables;
    use sonic_rs::Value;
    use std::collections::{BTreeSet, HashMap};

    fn value_from_number(n: i32) -> Value {
        sonic_rs::from_str(&n.to_string()).unwrap()
    }

    #[test]
    fn select_fetch_variables_only_used_variables() {
        let mut variable_values_map = HashMap::new();
        variable_values_map.insert("used".to_string(), value_from_number(1));
        variable_values_map.insert("unused".to_string(), value_from_number(2));
        let variable_values = Some(variable_values_map);

        let mut usages = BTreeSet::new();
        usages.insert("used".to_string());

        let selected = select_fetch_variables(&variable_values, Some(&usages)).unwrap();

        assert_eq!(selected.len(), 1);
        assert!(selected.contains_key("used"));
        assert!(!selected.contains_key("unused"));
    }

    #[test]
    fn select_fetch_variables_ignores_missing_usage_entries() {
        let mut variable_values_map = HashMap::new();
        variable_values_map.insert("present".to_string(), value_from_number(3));
        let variable_values = Some(variable_values_map);

        let mut usages = BTreeSet::new();
        usages.insert("present".to_string());
        usages.insert("missing".to_string());

        let selected = select_fetch_variables(&variable_values, Some(&usages)).unwrap();

        assert_eq!(selected.len(), 1);
        assert!(selected.contains_key("present"));
        assert!(!selected.contains_key("missing"));
    }

    #[test]
    fn select_fetch_variables_for_no_usage_entries() {
        let mut variable_values_map = HashMap::new();
        variable_values_map.insert("unused_1".to_string(), value_from_number(1));
        variable_values_map.insert("unused_2".to_string(), value_from_number(2));

        let variable_values = Some(variable_values_map);

        let selected = select_fetch_variables(&variable_values, None);

        assert!(selected.is_none());
    }
    #[test]
    /**
     * We have the same entity in two different paths ["a", 0] and ["b", 1],
     * and the subgraph response has an error for this entity.
     * So we should duplicate the error for both paths.
     */
    fn normalize_entity_errors_correctly() {
        use crate::response::graphql_error::{GraphQLError, GraphQLErrorPathSegment};
        use std::collections::HashMap;
        let mut ctx = ExecutionContext::default();
        let mut entity_index_error_map: HashMap<&usize, Vec<GraphQLErrorPath>> = HashMap::new();
        entity_index_error_map.insert(
            &0,
            vec![
                GraphQLErrorPath {
                    segments: vec![
                        GraphQLErrorPathSegment::String("a".to_string()),
                        GraphQLErrorPathSegment::Index(0),
                    ],
                },
                GraphQLErrorPath {
                    segments: vec![
                        GraphQLErrorPathSegment::String("b".to_string()),
                        GraphQLErrorPathSegment::Index(1),
                    ],
                },
            ],
        );
        let response_errors = vec![GraphQLError {
            message: "Error 1".to_string(),
            locations: None,
            path: Some(GraphQLErrorPath {
                segments: vec![
                    GraphQLErrorPathSegment::String("_entities".to_string()),
                    GraphQLErrorPathSegment::Index(0),
                    GraphQLErrorPathSegment::String("field1".to_string()),
                ],
            }),
            extensions: GraphQLErrorExtensions::default(),
        }];
        ctx.handle_errors(
            "subgraph_a",
            None,
            Some(response_errors),
            Some(entity_index_error_map),
        );
        assert_eq!(ctx.errors.len(), 2);
        assert_eq!(ctx.errors[0].message, "Error 1");
        assert_eq!(
            ctx.errors[0].path.as_ref().unwrap().segments,
            vec![
                GraphQLErrorPathSegment::String("a".to_string()),
                GraphQLErrorPathSegment::Index(0),
                GraphQLErrorPathSegment::String("field1".to_string())
            ]
        );
        assert_eq!(ctx.errors[1].message, "Error 1");
        assert_eq!(
            ctx.errors[1].path.as_ref().unwrap().segments,
            vec![
                GraphQLErrorPathSegment::String("b".to_string()),
                GraphQLErrorPathSegment::Index(1),
                GraphQLErrorPathSegment::String("field1".to_string())
            ]
        );
    }
}
