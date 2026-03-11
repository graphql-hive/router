use std::collections::{BTreeSet, HashMap};

use ahash::AHashMap;

use bytes::BufMut;
use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use hive_router_internal::telemetry::metrics::graphql_metrics::GraphQLErrorMetricsRecorder;
use hive_router_internal::telemetry::traces::spans::graphql::{
    GraphQLOperationSpan, GraphQLSpanOperationIdentity, GraphQLSubgraphOperationSpan,
};
use hive_router_query_planner::{
    ast::operation::OperationDefinition,
    planner::plan_nodes::{
        CompiledFieldProjectionPlan, ConditionNode, FetchNode, FetchRewrite, FlattenNodePath,
        PlanNode, QueryPlan, RuntimeCompiledSelectionSet, SchemaInterner,
    },
    state::supergraph_state::OperationKind,
};
use http::{HeaderMap, StatusCode};
use sonic_rs::ValueRef;
use tracing::Instrument;

use crate::{
    context::ExecutionContext,
    execution::{
        client_request_details::ClientRequestDetails,
        error::{IntoPlanExecutionError, LazyPlanContext, PlanExecutionError},
        jwt_forward::JwtAuthForwardingPlan,
    },
    executors::{common::SubgraphExecutionRequest, map::SubgraphExecutorMap},
    headers::{
        plan::{HeaderRulesPlan, ResponseHeaderAggregator},
        request::modify_subgraph_request_headers,
        response::apply_subgraph_response_headers,
    },
    hooks::{
        on_execute::{OnExecuteEndHookPayload, OnExecuteStartHookPayload},
        on_graphql_error::handle_graphql_errors_with_plugins,
    },
    introspection::{
        resolve::{resolve_introspection, IntrospectionContext},
        schema::SchemaMetadata,
    },
    plugin_context::PluginRequestState,
    plugin_trait::{EndControlFlow, StartControlFlow},
    projection::{
        request::{
            bind_runtime_selection_set, project_requires_flat, StaticRequiresRegistry,
            StaticRequiresSelectionSet,
        },
        response::project_by_operation,
    },
    response::{
        flat::{FlatResponseData, PathArena, PathNodeId, ValueId},
        graphql_error::GraphQLError,
        subgraph_response::SubgraphResponse,
    },
    utils::consts::{CLOSE_BRACKET, OPEN_BRACKET},
};

pub struct QueryPlanExecutionOpts<'exec> {
    pub query_plan: &'exec QueryPlan,
    pub operation_for_plan: &'exec OperationDefinition,
    pub projection_plan: &'exec Vec<CompiledFieldProjectionPlan>,
    pub static_projection_plan: &'exec Vec<crate::projection::response::StaticFieldProjectionPlan>,
    pub static_requires_registry: &'exec StaticRequiresRegistry,
    pub schema_interner: &'exec SchemaInterner,
    pub headers_plan: &'exec HeaderRulesPlan,
    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    pub extensions: HashMap<String, sonic_rs::Value>,
    pub client_request: &'exec ClientRequestDetails<'exec>,
    pub introspection_context: &'exec IntrospectionContext<'exec>,
    pub operation_type_name: &'exec str,
    pub executors: &'exec SubgraphExecutorMap,
    pub jwt_auth_forwarding: Option<JwtAuthForwardingPlan>,
    pub graphql_error_recorder: Option<GraphQLErrorMetricsRecorder>,
    pub initial_errors: Vec<GraphQLError>,
    pub span: &'exec GraphQLOperationSpan,
    pub plugin_req_state: &'exec Option<PluginRequestState<'exec>>,
}

#[derive(Default)]
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
        FlatResponseData::default()
    } else {
        FlatResponseData::empty_object()
    };

    let mut errors = opts.initial_errors;

    let mut extensions = opts.extensions;

    let mut query_plan = opts.query_plan;

    let dedupe_subgraph_requests = opts.operation_type_name == "Query";

    let mut on_end_callbacks = vec![];

    if let Some(plugin_req_state) = opts
        .plugin_req_state
        .as_ref()
        .filter(|plugin_req_state| !plugin_req_state.plugins.is_empty())
    {
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
        schema_interner: opts.schema_interner,
        static_requires_registry: opts.static_requires_registry,
        executors: opts.executors,
        client_request: opts.client_request,
        headers_plan: opts.headers_plan,
        jwt_forwarding_plan: opts.jwt_auth_forwarding,
        dedupe_subgraph_requests,
        plugin_req_state: opts.plugin_req_state,
    };

    if let Some(node) = &query_plan.node {
        executor.execute_plan_node(&mut exec_ctx, node).await;
    }

    let error_count = exec_ctx.errors.len(); // Added for usage reporting

    if error_count > 0 {
        opts.span.record_error_count(error_count);
        opts.span
            .record_errors(|| exec_ctx.errors.iter().map(|e| e.into()).collect());

        if let Some(error_recorder) = opts.graphql_error_recorder.as_ref() {
            error_recorder.record_errors(|| {
                exec_ctx
                    .errors
                    .iter()
                    .map(|err| err.extensions.code.as_deref())
            });
        }
    }

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

    let mut status_code = StatusCode::OK;

    if !errors.is_empty() {
        if let Some(plugin_req_state) = opts.plugin_req_state.as_ref() {
            let (new_errors, new_status_code) = handle_graphql_errors_with_plugins(
                plugin_req_state.plugins.as_ref(),
                errors,
                status_code,
            );

            errors = new_errors;
            status_code = new_status_code;
        }
    }

    let body = project_by_operation(
        &data,
        errors,
        &extensions,
        opts.operation_type_name,
        opts.static_projection_plan,
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
        status_code,
    })
}

pub struct Executor<'exec> {
    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    pub schema_metadata: &'exec SchemaMetadata,
    pub schema_interner: &'exec SchemaInterner,
    pub static_requires_registry: &'exec StaticRequiresRegistry,
    pub executors: &'exec SubgraphExecutorMap,
    pub client_request: &'exec ClientRequestDetails<'exec>,
    pub headers_plan: &'exec HeaderRulesPlan,
    pub jwt_forwarding_plan: Option<JwtAuthForwardingPlan>,
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
        fetch_node_id: i64,
        subgraph_name: &'exec str,
        response: SubgraphResponse<'exec>,
        flatten_node_path: &'exec FlattenNodePath,
        target_value_ids: Vec<ValueId>,
        target_error_path_ids: Vec<PathNodeId>,
        target_error_path_arena: PathArena,
        target_entity_indices: Vec<Option<usize>>,
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

impl<'exec> Executor<'exec> {
    async fn execute_plan_node(&self, ctx: &mut ExecutionContext<'exec>, node: &'exec PlanNode) {
        match node {
            PlanNode::Parallel(parallel_node) => {
                let mut scope = FuturesUnordered::new();

                for child in &parallel_node.nodes {
                    // We borrow `ctx.data` only for sync preparation of the job future,
                    // and the actual execution of the job future is done without the borrow of `ctx.data`
                    if let Some(fut) = self.prepare_job_future(child, &ctx.data) {
                        scope.push(fut);
                    }
                }

                while let Some(job) = scope.next().await {
                    self.process_job_result(ctx, job);
                }
            }
            PlanNode::Sequence(sequence_node) => {
                for child in &sequence_node.nodes {
                    // We use `Box.pin` here to avoid the compiler error about recursive future,
                    // as `execute_plan_node` is calling itself recursively for sequence nodes
                    Box::pin(self.execute_plan_node(ctx, child)).await;
                }
            }
            PlanNode::Condition(condition_node) => {
                if let Some(next_node) =
                    condition_node_by_variables(condition_node, self.variable_values)
                {
                    // We use `Box.pin` here to avoid the compiler error about recursive future,
                    // as `execute_plan_node` is calling itself recursively for condition nodes
                    Box::pin(self.execute_plan_node(ctx, next_node)).await;
                }
            }
            node => {
                if let Some(fut) = self.prepare_job_future(node, &ctx.data) {
                    let job = fut.await;
                    self.process_job_result(ctx, job);
                }
            }
        }
    }

    /**
     * This function is sync, because we only need the immutable borrow of `ctx.data` to prepare the subgraph request,
     * and the actual execution of the subgraph request is done in `prepare_fetch_job` which is async.
     * So we do everything in sync with `ctx.data` and return a future for the actual execution of the subgraph request.
     *
     * The return type is not a future of `Option`, but `Option` of future because the only case when we don't have a future,
     * and the result(`None`) is when the plan node is flatten node with no data.
     */
    fn prepare_job_future<'wave>(
        &'wave self,
        node: &'exec PlanNode,
        data: &FlatResponseData<'exec>,
    ) -> Option<BoxFuture<'wave, Result<ExecutionJob<'exec>, PlanExecutionError>>> {
        match node {
            PlanNode::Fetch(fetch_node) => {
                Some(self.prepare_fetch_job(fetch_node, None, None).boxed())
            }
            PlanNode::Flatten(flatten_node) => {
                let fetch_node = match flatten_node.node.as_ref() {
                    PlanNode::Fetch(fetch_node) => fetch_node,
                    _ => return None,
                };
                let static_requires = self.static_requires_registry.get(&fetch_node.id)?;
                let mut runtime_requires_cache: AHashMap<
                    (usize, u64),
                    RuntimeCompiledSelectionSet,
                > = AHashMap::new();
                let mut requires_type_name_cache = AHashMap::new();
                let typename_symbol = data.symbol_for("__typename");

                let normalized_path = flatten_node.path.as_slice();
                let mut filtered_representations = Vec::new();
                filtered_representations.put(OPEN_BRACKET);
                let possible_types = &self.schema_metadata.possible_types;
                let mut representation_hash_to_index: AHashMap<u64, usize> = AHashMap::new();
                let mut target_value_ids: Vec<ValueId> = Vec::new();
                let mut target_error_path_ids: Vec<PathNodeId> = Vec::new();
                let mut target_entity_indices: Vec<Option<usize>> = Vec::new();

                let flatten_targets =
                    data.collect_flatten_entities(normalized_path, possible_types);

                for ((entity_id, path_id), index_in_targets) in flatten_targets
                    .target_value_ids
                    .iter()
                    .zip(flatten_targets.path_ids.iter())
                    .zip(0..)
                {
                    let entity_id = *entity_id;
                    let path_id = *path_id;
                    let runtime_requires = get_or_bind_runtime_selection_set(
                        &mut runtime_requires_cache,
                        data,
                        static_requires,
                    );
                    let hash = data.hash_value_by_requires(
                        entity_id,
                        runtime_requires,
                        self.schema_interner,
                        possible_types,
                        &mut requires_type_name_cache,
                        typename_symbol,
                    );
                    let entity_index = representation_hash_to_index.get(&hash).copied();
                    let is_null = matches!(
                        data.value_kind(entity_id),
                        Some(crate::response::flat::FlatValue::Null)
                    );

                    if !is_null {
                        target_value_ids.push(entity_id);
                        target_error_path_ids.push(path_id);
                        target_entity_indices.push(entity_index);
                    }

                    if entity_index.is_some() {
                        continue;
                    }

                    let is_projected = if let Some(input_rewrites) = &fetch_node.input_rewrites {
                        let mut tmp = data.clone_subtree_as_root(entity_id);
                        for input_rewrite in input_rewrites {
                            tmp.apply_fetch_rewrite(possible_types, input_rewrite);
                        }
                        let tmp_runtime_requires = get_or_bind_runtime_selection_set(
                            &mut runtime_requires_cache,
                            &tmp,
                            static_requires,
                        );
                        project_requires_flat(
                            self.schema_interner,
                            &mut requires_type_name_cache,
                            &tmp,
                            possible_types,
                            tmp_runtime_requires,
                            tmp.root(),
                            &mut filtered_representations,
                            representation_hash_to_index.is_empty(),
                            None,
                        )
                    } else {
                        let runtime_requires = get_or_bind_runtime_selection_set(
                            &mut runtime_requires_cache,
                            data,
                            static_requires,
                        );
                        project_requires_flat(
                            self.schema_interner,
                            &mut requires_type_name_cache,
                            data,
                            possible_types,
                            runtime_requires,
                            entity_id,
                            &mut filtered_representations,
                            representation_hash_to_index.is_empty(),
                            None,
                        )
                    };

                    if is_projected {
                        let projected_index = index_in_targets;
                        representation_hash_to_index.insert(hash, projected_index);
                        if !is_null {
                            if let Some(last) = target_entity_indices.last_mut() {
                                *last = Some(projected_index);
                            }
                        }
                    }
                }

                filtered_representations.put(CLOSE_BRACKET);

                if representation_hash_to_index.is_empty() {
                    return None;
                }

                // This is the future for the actual fetch job
                Some(
                    async {
                        let fetch_job = self
                            .prepare_fetch_job(
                                fetch_node,
                                Some(filtered_representations),
                                Some(&flatten_node.path),
                            )
                            .await?;
                        Ok(ExecutionJob::FlattenFetch {
                            flatten_node_path: &flatten_node.path,
                            response: fetch_job.response(),
                            fetch_node_id: fetch_node.id,
                            subgraph_name: fetch_node.service_name.as_str(),
                            target_value_ids,
                            target_error_path_ids,
                            target_error_path_arena: flatten_targets.path_arena,
                            target_entity_indices,
                        })
                    }
                    .boxed(),
                )
            }
            PlanNode::Condition(node) => condition_node_by_variables(node, self.variable_values)
                .and_then(|node| self.prepare_job_future(node, data)),
            // Our Query Planner does not produce any other plan node types in ParallelNode
            _ => None,
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
        job: Result<ExecutionJob<'exec>, PlanExecutionError>,
    ) {
        match job {
            Err(err) => {
                self.log_error(&err);
                ctx.errors.push(err.into());
            }
            Ok(job) => {
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
                        self.log_error(&err);
                        ctx.errors.push(err.into());
                    }
                }

                let output_rewrites: Option<&[FetchRewrite]> =
                    ctx.output_rewrites.get(job.fetch_node_id());

                let (errors, entity_index_error_map) = match job {
                    ExecutionJob::Fetch { response, .. } => {
                        if let Some(response_bytes) = response.bytes {
                            ctx.response_storage.add_response(response_bytes);
                        }
                        if let Some(output_rewrites) = output_rewrites {
                            let mut response_data = response.data;
                            for output_rewrite in output_rewrites {
                                response_data.apply_fetch_rewrite(
                                    &self.schema_metadata.possible_types,
                                    output_rewrite,
                                );
                            }
                            ctx.data.merge_from(&response_data);
                        } else {
                            ctx.data.merge_from(&response.data);
                        }

                        (response.errors, None)
                    }
                    ExecutionJob::FlattenFetch {
                        mut response,
                        target_value_ids,
                        target_error_path_ids,
                        target_entity_indices,
                        target_error_path_arena,
                        ..
                    } => {
                        if let Some(response_bytes) = response.bytes {
                            ctx.response_storage.add_response(response_bytes);
                        }
                        if let Some(entity_value_ids) = response.data.entities_value_ids() {
                            let entity_value_ids = entity_value_ids.to_vec();
                            if let Some(output_rewrites) = output_rewrites {
                                for output_rewrite in output_rewrites {
                                    for entity_value_id in &entity_value_ids {
                                        response.data.apply_fetch_rewrite_at_value(
                                            &self.schema_metadata.possible_types,
                                            *entity_value_id,
                                            output_rewrite,
                                        );
                                    }
                                }
                            }

                            let mut entity_index_error_map = response
                                .errors
                                .as_ref()
                                .map(|_| HashMap::with_capacity(target_error_path_ids.len()));

                            for ((path_id, target_value_id), maybe_entity_index) in
                                target_error_path_ids
                                    .into_iter()
                                    .zip(target_value_ids.into_iter())
                                    .zip(target_entity_indices.into_iter())
                            {
                                let Some(entity_index) = maybe_entity_index else {
                                    continue;
                                };

                                if let Some(entity_index_error_map) =
                                    entity_index_error_map.as_mut()
                                {
                                    let error_paths = entity_index_error_map
                                        .entry(entity_index)
                                        .or_insert_with(Vec::new);
                                    error_paths.push(target_error_path_arena.materialize(path_id));
                                }

                                if let Some(entity_value_id) = entity_value_ids.get(entity_index) {
                                    ctx.data.deep_merge_values(
                                        target_value_id,
                                        &response.data,
                                        *entity_value_id,
                                    );
                                }
                            }
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

    fn log_error(&self, error: &PlanExecutionError) {
        if let Some(subgraph_name) = error.subgraph_name() {
            tracing::error!(
                "Error executing plan with subgraph '{}': {}",
                subgraph_name,
                error
            );
        } else {
            tracing::error!("Error executing plan: {}", error);
        }
    }

    async fn prepare_fetch_job(
        &self,
        node: &'exec FetchNode,
        // If the fetch job is for a flatten node, we pass the filtered representations,
        representations: Option<Vec<u8>>,
        // and the path to the representations in the original response for error handling and normalization
        affected_path: Option<&FlattenNodePath>,
    ) -> Result<ExecutionJob<'exec>, PlanExecutionError> {
        let subgraph_operation_span = GraphQLSubgraphOperationSpan::new(
            node.service_name.as_str(),
            &node.operation.document_str,
        );

        async {
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
                schema_interner: self.schema_interner,
            };

            subgraph_operation_span.record_operation_identity(GraphQLSpanOperationIdentity {
                name: subgraph_request.operation_name,
                operation_type: match node.operation_kind {
                    Some(OperationKind::Query) | None => "query",
                    Some(OperationKind::Mutation) => "mutation",
                    Some(OperationKind::Subscription) => "subscription",
                },
                client_document_hash: node.operation.hash.to_string().as_str(),
            });

            if let Some(jwt_forwarding_plan) = &self.jwt_forwarding_plan {
                subgraph_request.add_request_extensions_field(
                    jwt_forwarding_plan.extension_field_name.clone(),
                    jwt_forwarding_plan.extension_field_value.clone(),
                );
            }

            let response = self
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
                })?;

            if let Some(errors) = &response.errors {
                if !errors.is_empty() {
                    subgraph_operation_span.record_error_count(errors.len());
                    subgraph_operation_span
                        .record_errors(|| errors.iter().map(|e| e.into()).collect());
                }
            }

            Ok(ExecutionJob::Fetch {
                fetch_node_id: node.id,
                subgraph_name: &node.service_name,
                response,
            })
        }
        .instrument(subgraph_operation_span.clone())
        .await
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

fn get_or_bind_runtime_selection_set<'a>(
    cache: &'a mut AHashMap<(usize, u64), RuntimeCompiledSelectionSet>,
    data: &FlatResponseData,
    selection_set: &StaticRequiresSelectionSet,
) -> &'a RuntimeCompiledSelectionSet {
    let key = (
        selection_set as *const StaticRequiresSelectionSet as usize,
        data.symbol_generation(),
    );
    cache
        .entry(key)
        .or_insert_with(|| bind_runtime_selection_set(data, selection_set))
}

#[cfg(test)]
mod tests {
    use crate::{
        context::ExecutionContext,
        execution::{
            client_request_details::{ClientRequestDetails, JwtRequestDetails, OperationDetails},
            plan::Executor,
        },
        headers::plan::HeaderRulesPlan,
        introspection::schema::SchemaMetadata,
        response::flat::FlatResponseData,
        response::graphql_error::{GraphQLErrorExtensions, GraphQLErrorPath},
        SubgraphExecutorMap,
    };

    use super::select_fetch_variables;
    use hive_router_config::HiveRouterConfig;
    use hive_router_internal::telemetry::TelemetryContext;
    use hive_router_query_planner::{
        ast::{
            document::Document,
            operation::{OperationDefinition, SubgraphFetchOperation},
            selection_set::SelectionSet,
        },
        planner::plan_nodes::{FetchNode, ParallelNode, PlanNode},
    };
    use ntex::http::HeaderMap;
    use sonic_rs::Value;
    use std::{
        collections::{BTreeSet, HashMap},
        sync::{mpsc::channel, Arc},
        time::Duration,
        vec,
    };

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
        let mut entity_index_error_map: HashMap<usize, Vec<GraphQLErrorPath>> = HashMap::new();
        entity_index_error_map.insert(
            0,
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

    #[tokio::test]
    async fn runs_parallel_jobs_in_parallel() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut subgraph_a = mockito::Server::new_async().await;
        let mut subgraph_b = mockito::Server::new_async().await;
        let data = FlatResponseData::default();
        let subgraph_endpoint_map = HashMap::from([
            (
                "subgraph_a".to_string(),
                format!("http://{}/graphql", subgraph_a.host_with_port())
                    .parse()
                    .unwrap(),
            ),
            (
                "subgraph_b".to_string(),
                format!("http://{}/graphql", subgraph_b.host_with_port())
                    .parse()
                    .unwrap(),
            ),
        ]);
        let static_requires_registry = super::StaticRequiresRegistry::new();
        let executor = Executor {
            variable_values: &None,
            schema_metadata: &SchemaMetadata::default(),
            schema_interner:
                &hive_router_query_planner::planner::plan_nodes::SchemaInterner::default(),
            static_requires_registry: &static_requires_registry,
            executors: &SubgraphExecutorMap::from_http_endpoint_map(
                &subgraph_endpoint_map,
                HiveRouterConfig::default().into(),
                Arc::new(TelemetryContext::from_propagation_config(
                    &Default::default(),
                )),
            )
            .unwrap(),
            client_request: &ClientRequestDetails {
                method: &http::Method::POST,
                url: &"http://example.com".parse().unwrap(),
                headers: &HeaderMap::new(),
                operation: OperationDetails {
                    name: None,
                    query: "{ from_a from_b }",
                    kind: "query",
                },
                jwt: JwtRequestDetails::Unauthenticated,
            },
            headers_plan: &HeaderRulesPlan::default(),
            jwt_forwarding_plan: None,
            dedupe_subgraph_requests: false,
            plugin_req_state: &None,
        };

        let mock_a = subgraph_a
            .mock("POST", "/graphql")
            .with_body(r#"{"data":{"from_a":"value_a"}}"#)
            .create();

        let mut exec_ctx = ExecutionContext {
            data,
            ..Default::default()
        };

        // It is ok to have 'static lifetime here, because `data` is owned by `exec_ctx`, and `exec_ctx` lives for the entire duration of the test,
        // so the reference to `data` will never be dangling.
        let data_ref: &'static FlatResponseData<'static> =
            unsafe { std::mem::transmute(&exec_ctx.data) };

        let (sender, receiver) = channel();

        let mock_b = subgraph_b
            .mock("POST", "/graphql")
            .with_chunked_body(move |writer| {
                // We can add some delay here to make sure the parallel execution is actually working
                std::thread::sleep(Duration::from_millis(1000));
                // data should have `from_a` field from subgraph_a's response,
                // so data the merging process does not wait for subgraph_b's response to merge subgraph_a's response
                if let Some(from_a_value) = data_ref
                    .object_field(data_ref.root(), "from_a")
                    .and_then(|id| data_ref.value_as_str(id))
                {
                    sender
                        .send(from_a_value.to_string())
                        .expect("Failed to send from_a value through channel");
                }
                writer.write_fmt(format_args!(r#"{{"data":{{"from_b":"value_b"}}}}"#))
            })
            .create();

        let dummy_doc = Document {
            operation: OperationDefinition {
                name: None,
                operation_kind: None,
                variable_definitions: None,
                selection_set: SelectionSet { items: vec![] },
            },

            fragments: vec![],
        };

        executor
            .execute_plan_node(
                &mut exec_ctx,
                &PlanNode::Parallel(ParallelNode {
                    nodes: vec![
                        PlanNode::Fetch(FetchNode {
                            id: 1,
                            service_name: "subgraph_a".to_string(),
                            operation: SubgraphFetchOperation {
                                document_str: "{ from_a }".to_string(),
                                document: dummy_doc.clone(),
                                hash: 0,
                            },
                            operation_name: None,
                            requires: None,
                            compiled_requires: None,
                            input_rewrites: None,
                            output_rewrites: None,
                            variable_usages: None,
                            operation_kind: None,
                        }),
                        PlanNode::Fetch(FetchNode {
                            id: 2,
                            service_name: "subgraph_b".to_string(),
                            operation: SubgraphFetchOperation {
                                document_str: "{ from_b }".to_string(),
                                document: dummy_doc.clone(),
                                hash: 0,
                            },
                            operation_name: None,
                            requires: None,
                            compiled_requires: None,
                            input_rewrites: None,
                            output_rewrites: None,
                            variable_usages: None,
                            operation_kind: None,
                        }),
                    ],
                }),
            )
            .await;
        mock_a.assert();
        mock_b.assert();

        let from_a_value = receiver
            .recv()
            .expect("Failed to receive from_a value through channel");
        assert_eq!(from_a_value, "value_a");
    }
}
