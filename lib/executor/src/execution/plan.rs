use std::collections::{BTreeSet, HashMap};

use bytes::BufMut;
use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use hive_router_query_planner::{
    ast::operation::OperationDefinition,
    planner::plan_nodes::{
        ConditionNode, FetchNode, FetchRewrite, FlattenNode, FlattenNodePath, PlanNode, QueryPlan,
    },
};
use http::{HeaderMap, StatusCode};
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
        plan::{HeaderRulesPlan, ResponseHeaderAggregator},
        request::modify_subgraph_request_headers,
        response::apply_subgraph_response_headers,
    },
    hooks::on_execute::{OnExecuteEndHookPayload, OnExecuteStartHookPayload},
    introspection::{
        resolve::{resolve_introspection, IntrospectionContext},
        schema::SchemaMetadata,
    },
    plugin_context::PluginRequestState,
    plugin_trait::{EndControlFlow, StartControlFlow},
    projection::{
        plan::FieldProjectionPlan, request::project_requires, response::project_by_operation,
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

pub struct QueryPlanExecutionOpts<'exec> {
    pub plugin_req_state: &'exec Option<PluginRequestState<'exec>>,
    pub query_plan: &'exec QueryPlan,
    pub operation_for_plan: &'exec OperationDefinition,
    pub projection_plan: &'exec Vec<FieldProjectionPlan>,
    pub headers_plan: &'exec HeaderRulesPlan,
    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    pub extensions: HashMap<String, sonic_rs::Value>,
    pub client_request: &'exec ClientRequestDetails<'exec>,
    pub introspection_context: &'exec IntrospectionContext<'exec, 'static>,
    pub operation_type_name: &'exec str,
    pub executors: &'exec SubgraphExecutorMap,
    pub jwt_auth_forwarding: Option<JwtAuthForwardingPlan>,
    pub errors: Vec<GraphQLError>,
}

pub struct PlanExecutionOutput {
    pub body: Vec<u8>,
    pub response_headers_aggregator: Option<ResponseHeaderAggregator>,
    pub error_count: usize,
    pub status_code: StatusCode,
}

pub async fn execute_query_plan<'exec>(
    opts: QueryPlanExecutionOpts<'exec>,
) -> Result<PlanExecutionOutput, PlanExecutionError> {
    let mut data = if let Some(introspection_query) = opts.introspection_context.query {
        resolve_introspection(introspection_query, opts.introspection_context)
    } else if opts.projection_plan.is_empty() {
        Value::Null
    } else {
        Value::Object(Vec::new())
    };

    let mut errors = opts.errors;

    let mut extensions = opts.extensions;

    let mut query_plan = opts.query_plan;

    let dedupe_subgraph_requests = opts.operation_type_name == "Query";

    let mut on_end_callbacks = vec![];

    if let Some(plugin_req_state) = opts.plugin_req_state.as_ref() {
        let mut start_payload = OnExecuteStartHookPayload {
            router_http_request: &plugin_req_state.router_http_request,
            context: &plugin_req_state.context,
            query_plan,
            operation_for_plan: opts.operation_for_plan,
            data,
            errors,
            extensions,
            variable_values: opts.variable_values,
            dedupe_subgraph_requests,
        };

        for plugin in plugin_req_state.plugins.as_ref() {
            let result = plugin.on_execute(start_payload).await;
            start_payload = result.payload;
            match result.control_flow {
                StartControlFlow::Proceed => { /* continue to next plugin */ }
                StartControlFlow::EndWithResponse(response) => {
                    return Ok(response);
                }
                StartControlFlow::OnEnd(callback) => {
                    on_end_callbacks.push(callback);
                }
            }
        }

        // Give the ownership back to variables
        query_plan = start_payload.query_plan;
        data = start_payload.data;
        errors = start_payload.errors;
        extensions = start_payload.extensions;
    }

    let mut exec_ctx = ExecutionContext::new(query_plan, data, errors);
    // No need for `new`, it has too many parameters
    // We can directly create `Executor` instance here
    let executor = Executor {
        variable_values: opts.variable_values,
        schema_metadata: opts.introspection_context.metadata,
        executors: opts.executors,
        client_request: opts.client_request,
        headers_plan: opts.headers_plan,
        jwt_forwarding_plan: &opts.jwt_auth_forwarding,
        dedupe_subgraph_requests,
        plugin_req_state: opts.plugin_req_state,
    };

    if let Some(node) = &query_plan.node {
        executor.execute(&mut exec_ctx, node).await;
    }

    let error_count = exec_ctx.errors.len(); // Added for usage reporting

    let mut data = exec_ctx.data;
    let mut errors = exec_ctx.errors;
    let mut response_size_estimate = exec_ctx.response_storage.estimate_final_response_size();

    if !on_end_callbacks.is_empty() {
        let mut end_payload = OnExecuteEndHookPayload {
            data,
            errors,
            extensions,
            response_size_estimate,
        };

        for callback in on_end_callbacks {
            let result = callback(end_payload);
            end_payload = result.payload;
            match result.control_flow {
                EndControlFlow::Proceed => { /* continue to next callback */ }
                EndControlFlow::EndWithResponse(response) => {
                    return Ok(response);
                }
            }
        }

        // Give the ownership back to variables
        data = end_payload.data;
        errors = end_payload.errors;
        extensions = end_payload.extensions;
        response_size_estimate = end_payload.response_size_estimate;
    }

    let body = project_by_operation(
        &data,
        errors,
        &extensions,
        opts.operation_type_name,
        opts.projection_plan,
        opts.variable_values,
        response_size_estimate,
        opts.introspection_context.metadata,
    )
    .with_plan_context(LazyPlanContext {
        subgraph_name: || None,
        affected_path: || None,
    })?;

    Ok(PlanExecutionOutput {
        body,
        response_headers_aggregator: exec_ctx.response_headers_aggregator.none_if_empty(),
        error_count,
        status_code: StatusCode::OK,
    })
}

pub struct Executor<'exec> {
    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    pub schema_metadata: &'exec SchemaMetadata,
    pub executors: &'exec SubgraphExecutorMap,
    pub client_request: &'exec ClientRequestDetails<'exec>,
    pub headers_plan: &'exec HeaderRulesPlan,
    pub jwt_forwarding_plan: &'exec Option<JwtAuthForwardingPlan>,
    pub dedupe_subgraph_requests: bool,
    pub plugin_req_state: &'exec Option<PluginRequestState<'exec>>,
}

enum ExecutionJob<'exec> {
    Fetch {
        fetch_node_id: i64,
        subgraph_name: &'exec str,
        response: SubgraphResponse<'exec>,
    },
    FlattenFetch {
        flatten_node_path: &'exec FlattenNodePath,
        response: SubgraphResponse<'exec>,
        fetch_node_id: i64,
        subgraph_name: &'exec str,
        representation_hashes: Vec<u64>,
        representation_hash_to_index: HashMap<u64, usize>,
    },
}

impl<'exec> ExecutionJob<'exec> {
    fn response(self) -> SubgraphResponse<'exec> {
        match self {
            ExecutionJob::Fetch { response, .. } => response,
            ExecutionJob::FlattenFetch { response, .. } => response,
        }
    }
    fn response_ref(&self) -> &SubgraphResponse<'exec> {
        match self {
            ExecutionJob::Fetch { response, .. } => response,
            ExecutionJob::FlattenFetch { response, .. } => response,
        }
    }
    fn fetch_node_id(&self) -> i64 {
        match self {
            ExecutionJob::Fetch { fetch_node_id, .. } => *fetch_node_id,
            ExecutionJob::FlattenFetch { fetch_node_id, .. } => *fetch_node_id,
        }
    }
    fn subgraph_name(&self) -> &'exec str {
        match self {
            ExecutionJob::Fetch { subgraph_name, .. } => subgraph_name,
            ExecutionJob::FlattenFetch { subgraph_name, .. } => subgraph_name,
        }
    }
    fn affected_path(&self) -> Option<&'exec FlattenNodePath> {
        match self {
            ExecutionJob::Fetch { .. } => None,
            ExecutionJob::FlattenFetch {
                flatten_node_path, ..
            } => Some(flatten_node_path),
        }
    }
}

struct PreparedFlattenData {
    representations: Vec<u8>,
    representation_hashes: Vec<u64>,
    representation_hash_to_index: HashMap<u64, usize>,
}

impl<'exec> Executor<'exec> {
    async fn execute(&self, ctx: &mut ExecutionContext<'exec>, node: &'exec PlanNode) {
        match node {
            PlanNode::Parallel(parallel_node) => {
                let mut scope = FuturesUnordered::new();

                for child in &parallel_node.nodes {
                    let fut = self.prepare_job_future(child, &ctx.data);
                    scope.push(fut);
                }

                while let Some(job) = scope.next().await {
                    self.process_job_result(ctx, job);
                }
            }
            PlanNode::Sequence(sequence_node) => {
                for child in &sequence_node.nodes {
                    Box::pin(self.execute(ctx, child)).await;
                }
            }
            PlanNode::Condition(condition_node) => {
                match condition_node_by_variables(condition_node, self.variable_values) {
                    Some(node) => Box::pin(self.execute(ctx, node)).await,
                    None => { /* No-op */ }
                }
            }
            node => {
                let job = self.prepare_job_future(node, &ctx.data).await;
                self.process_job_result(ctx, job);
            }
        }
    }

    fn prepare_job_future<'wave>(
        &'wave self,
        node: &'exec PlanNode,
        data: &Value<'exec>,
    ) -> BoxFuture<'wave, Result<Option<ExecutionJob<'exec>>, PlanExecutionError>> {
        match node {
            PlanNode::Fetch(fetch_node) => self.execute_fetch_node(fetch_node, None, None).boxed(),
            PlanNode::Flatten(flatten_node) => {
                match self.prepare_flatten_data(data, flatten_node) {
                    Ok(Some(p)) => async {
                        if let PlanNode::Fetch(fetch_node) = flatten_node.node.as_ref() {
                            if let Some(fetch_job) = self
                                .execute_fetch_node(
                                    fetch_node,
                                    Some(p.representations),
                                    Some(&flatten_node.path),
                                )
                                .await?
                            {
                                return Ok(Some(ExecutionJob::FlattenFetch {
                                    flatten_node_path: &flatten_node.path,
                                    response: fetch_job.response(),
                                    fetch_node_id: fetch_node.id,
                                    subgraph_name: fetch_node.service_name.as_str(),
                                    representation_hashes: p.representation_hashes,
                                    representation_hash_to_index: p.representation_hash_to_index,
                                }));
                            }
                        }
                        Ok(None)
                    }
                    .boxed(),
                    Ok(None) => async { Ok(None) }.boxed(),
                    Err(e) => async { Err(e) }.boxed(),
                }
            }
            PlanNode::Condition(node) => {
                match condition_node_by_variables(node, self.variable_values) {
                    Some(node) => self.prepare_job_future(node, data), // This is already clean.
                    None => async { Ok(None) }.boxed(),
                }
            }
            // Our Query Planner does not produce any other plan node types in ParallelNode
            _ => async { Ok(None) }.boxed(),
        }
    }

    // We handle `Result` instead of passing `PlanExecutionError` directly
    // as PipelineError so the first occurrence of an error does not stop the whole execution
    // But those errors are added to the final GraphQL response in `errors` field
    // of the GraphQL response
    // For example, if a subgraph is down, the rest of the plan can still be executed
    // See `error_handling_e2e_tests` for reproduction
    fn process_job_result(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        job: Result<Option<ExecutionJob<'exec>>, PlanExecutionError>,
    ) {
        match job {
            Ok(None) => { /* No-op */ }
            Err(err) => {
                ctx.errors.push(err.into());
            }
            Ok(Some(job)) => {
                let subgraph_name = job.subgraph_name();
                let affected_path = job.affected_path();
                if let Some(ref subgraph_headers) = job.response_ref().headers {
                    if let Err(err) = apply_subgraph_response_headers(
                        self.headers_plan,
                        job.subgraph_name(),
                        subgraph_headers,
                        self.client_request,
                        &mut ctx.response_headers_aggregator,
                    )
                    .with_plan_context(LazyPlanContext {
                        subgraph_name: || Some(subgraph_name.to_string()),
                        affected_path: || affected_path.map(|p| p.to_string()),
                    }) {
                        ctx.errors.push(err.into());
                    }
                }

                let output_rewrites: Option<&[FetchRewrite]> =
                    ctx.output_rewrites.get(job.fetch_node_id());

                let (errors, entity_index_error_map) = match job {
                    ExecutionJob::Fetch { mut response, .. } => {
                        if let Some(response_bytes) = response.bytes {
                            ctx.response_storage.add_response(response_bytes);
                        }
                        if let Some(output_rewrites) = output_rewrites {
                            for output_rewrite in output_rewrites {
                                output_rewrite.rewrite(
                                    &self.schema_metadata.possible_types,
                                    &mut response.data,
                                );
                            }
                        }
                        deep_merge(&mut ctx.data, response.data);

                        (response.errors, None)
                    }
                    ExecutionJob::FlattenFetch {
                        mut response,
                        flatten_node_path,
                        representation_hashes,
                        ref representation_hash_to_index,
                        ..
                    } => {
                        if let Some(response_bytes) = response.bytes {
                            ctx.response_storage.add_response(response_bytes);
                        }
                        if let Some(mut entities) = response.data.take_entities() {
                            if let Some(output_rewrites) = output_rewrites {
                                for output_rewrite in output_rewrites {
                                    for entity in &mut entities {
                                        output_rewrite
                                            .rewrite(&self.schema_metadata.possible_types, entity);
                                    }
                                }
                            }

                            let mut index = 0;
                            let normalized_path = flatten_node_path.as_slice();
                            // If there is an error in the response, then collect the paths for normalizing the error
                            let initial_error_path = response.errors.as_ref().map(|_| {
                                GraphQLErrorPath::with_capacity(normalized_path.len() + 2)
                            });
                            let mut entity_index_error_map = response
                                .errors
                                .as_ref()
                                .map(|_| HashMap::with_capacity(entities.len()));
                            traverse_and_callback_mut(
                                &mut ctx.data,
                                normalized_path,
                                self.schema_metadata,
                                initial_error_path,
                                &mut |target, error_path| {
                                    let hash = representation_hashes[index];
                                    if let Some(entity_index) =
                                        representation_hash_to_index.get(&hash)
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
                                            // SAFETY: `new_val` is a clone of an entity that lives for `'a`.
                                            // The transmute is to satisfy the compiler, but the lifetime
                                            // is valid.
                                            let new_val: Value<'_> =
                                                unsafe { std::mem::transmute(entity.clone()) };
                                            deep_merge(target, new_val);
                                        }
                                    }
                                    index += 1;
                                },
                            );
                            (response.errors, entity_index_error_map)
                        } else {
                            (response.errors, None)
                        }
                    }
                };

                ctx.handle_errors(subgraph_name, affected_path, errors, entity_index_error_map);
            }
        }
    }

    #[inline]
    fn prepare_flatten_data(
        &self,
        data: &Value<'exec>,
        flatten_node: &FlattenNode,
    ) -> Result<Option<PreparedFlattenData>, PlanExecutionError> {
        let fetch_node = match flatten_node.node.as_ref() {
            PlanNode::Fetch(fetch_node) => fetch_node,
            _ => return Ok(None),
        };
        let requires_nodes = match fetch_node.requires.as_ref() {
            Some(nodes) => nodes,
            None => return Ok(None),
        };

        let mut index = 0;
        let normalized_path = flatten_node.path.as_slice();
        let mut filtered_representations = Vec::new();
        filtered_representations.put(OPEN_BRACKET);
        let possible_types = &self.schema_metadata.possible_types;
        let mut representation_hashes: Vec<u64> = Vec::new();
        let mut filtered_representations_hashes: HashMap<u64, usize> = HashMap::new();
        let arena = bumpalo::Bump::new();

        traverse_and_callback(data, normalized_path, self.schema_metadata, &mut |entity| {
            let hash = entity.to_hash(&requires_nodes.items, possible_types);

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
                possible_types,
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
        })?;
        filtered_representations.put(CLOSE_BRACKET);

        if filtered_representations_hashes.is_empty() {
            return Ok(None);
        }

        Ok(Some(PreparedFlattenData {
            representations: filtered_representations,
            representation_hashes,
            representation_hash_to_index: filtered_representations_hashes,
        }))
    }

    async fn execute_fetch_node(
        &self,
        node: &'exec FetchNode,
        representations: Option<Vec<u8>>,
        affected_path: Option<&FlattenNodePath>,
    ) -> Result<Option<ExecutionJob<'exec>>, PlanExecutionError> {
        // TODO: We could optimize header map creation by caching them per service name
        let mut headers_map = HeaderMap::new();
        let subgraph_name_factory = || Some(node.service_name.clone());
        let affected_path_factory = || affected_path.map(|p| p.to_string());
        modify_subgraph_request_headers(
            self.headers_plan,
            &node.service_name,
            self.client_request,
            &mut headers_map,
        )
        .with_plan_context(LazyPlanContext {
            subgraph_name: subgraph_name_factory,
            affected_path: affected_path_factory,
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

        Ok(Some(ExecutionJob::Fetch {
            fetch_node_id: node.id,
            subgraph_name: &node.service_name,
            response: self
                .executors
                .execute(
                    &node.service_name,
                    subgraph_request,
                    self.client_request,
                    self.plugin_req_state,
                )
                .await
                .with_plan_context(LazyPlanContext {
                    subgraph_name: subgraph_name_factory,
                    affected_path: affected_path_factory,
                })?,
        }))
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
