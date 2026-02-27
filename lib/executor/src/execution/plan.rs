use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use ahash::{HashMap as AHashMap, HashMapExt};
use bytes::BufMut;
use futures::TryFutureExt;
use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use hive_router_internal::telemetry::metrics::graphql_metrics::GraphQLErrorMetricsRecorder;
use hive_router_internal::telemetry::traces::spans::graphql::{
    GraphQLOperationSpan, GraphQLSpanOperationIdentity, GraphQLSubgraphOperationSpan,
};
use hive_router_query_planner::ast::operation::SubgraphFetchOperation;
use hive_router_query_planner::{
    ast::operation::OperationDefinition,
    planner::plan_nodes::{
        ConditionNode, EntityBatch, EntityBatchAlias, FetchRewrite, FlattenNodePath, PlanNode,
        QueryPlan,
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
        rewrites::FetchRewriteExt,
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
        plan::FieldProjectionPlan, request::project_requires, response::project_by_operation,
    },
    response::{
        graphql_error::{GraphQLError, GraphQLErrorPath, GraphQLErrorPathSegment},
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
    pub query_plan: &'exec QueryPlan,
    pub operation_for_plan: &'exec OperationDefinition,
    pub projection_plan: &'exec Vec<FieldProjectionPlan>,
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
        Value::Null
    } else {
        Value::Object(Vec::new())
    };

    let mut errors = opts.initial_errors;

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

    let mut exec_ctx = ExecutionContext::new(data, errors);
    // No need for `new`, it has too many parameters
    // We can directly create `Executor` instance here
    let executor = Executor {
        variable_values: opts.variable_values,
        schema_metadata: opts.introspection_context.metadata,
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
        status_code,
    })
}

pub struct Executor<'exec> {
    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    pub schema_metadata: &'exec SchemaMetadata,
    pub executors: &'exec SubgraphExecutorMap,
    pub client_request: &'exec ClientRequestDetails<'exec>,
    pub headers_plan: &'exec HeaderRulesPlan,
    pub jwt_forwarding_plan: Option<JwtAuthForwardingPlan>,
    pub dedupe_subgraph_requests: bool,
    pub plugin_req_state: &'exec Option<PluginRequestState<'exec>>,
}

enum ExecutionJob<'exec> {
    Fetch {
        subgraph_name: &'exec str,
        response: SubgraphResponse<'exec>,
        output_rewrites: Option<&'exec [FetchRewrite]>,
    },
    FlattenFetch {
        subgraph_name: &'exec str,
        response: SubgraphResponse<'exec>,
        flatten_node_path: &'exec FlattenNodePath,
        representation_hashes: Vec<u64>,
        representation_hash_to_index: AHashMap<u64, usize>,
        output_rewrites: Option<&'exec [FetchRewrite]>,
    },
    BatchFetch {
        subgraph_name: &'exec str,
        response: SubgraphResponse<'exec>,
        aliases: Vec<AliasBatchState<'exec>>,
    },
}

struct AliasBatchState<'exec> {
    alias_spec: &'exec EntityBatchAlias,
    representation_hash_to_index: AHashMap<u64, usize>,
    paths: Vec<AliasPathState<'exec>>,
}

struct AliasPathState<'exec> {
    merge_path: &'exec FlattenNodePath,
    representation_hashes: Arc<Vec<u64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct AliasIndex(usize);

#[derive(Default)]
struct BatchFetchErrors {
    by_alias_index: AHashMap<AliasIndex, Vec<GraphQLError>>,
    unmatched: Vec<GraphQLError>,
}

impl<'exec> ExecutionJob<'exec> {
    fn response(self) -> SubgraphResponse<'exec> {
        match self {
            ExecutionJob::Fetch { response, .. } => response,
            ExecutionJob::FlattenFetch { response, .. } => response,
            ExecutionJob::BatchFetch { response, .. } => response,
        }
    }
    fn response_ref(&self) -> &SubgraphResponse<'exec> {
        match self {
            ExecutionJob::Fetch { response, .. } => response,
            ExecutionJob::FlattenFetch { response, .. } => response,
            ExecutionJob::BatchFetch { response, .. } => response,
        }
    }
    fn subgraph_name(&self) -> &'exec str {
        match self {
            ExecutionJob::Fetch { subgraph_name, .. } => subgraph_name,
            ExecutionJob::FlattenFetch { subgraph_name, .. } => subgraph_name,
            ExecutionJob::BatchFetch { subgraph_name, .. } => subgraph_name,
        }
    }
    fn affected_path(&self) -> Option<&'exec FlattenNodePath> {
        match self {
            ExecutionJob::Fetch { .. } => None,
            ExecutionJob::FlattenFetch {
                flatten_node_path, ..
            } => Some(flatten_node_path),
            ExecutionJob::BatchFetch { .. } => None,
        }
    }
}

struct PrepareExecutionJobOpts<'exec> {
    // The name of the subgraph
    subgraph_name: &'exec str,
    // Variable usages
    variable_usages: Option<&'exec BTreeSet<String>>,
    // Operation name
    operation_name: Option<&'exec str>,
    // Operation Kind
    operation_kind: Option<&'exec OperationKind>,
    // Operation
    operation: &'exec SubgraphFetchOperation,
    // Output rewrites
    output_rewrites: Option<&'exec [FetchRewrite]>,
    // If the fetch job is for a flatten node, we pass the filtered representations,
    raw_variable_values: Option<Vec<(&'exec str, Vec<u8>)>>,
    // and the path to the representations in the original response for error handling and normalization
    affected_path: Option<&'exec FlattenNodePath>,
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
        data: &Value<'exec>,
    ) -> Option<BoxFuture<'wave, Result<ExecutionJob<'exec>, PlanExecutionError>>> {
        match node {
            PlanNode::Fetch(fetch_node) => Some(
                self.prepare_execution_job(PrepareExecutionJobOpts {
                    subgraph_name: &fetch_node.service_name,
                    variable_usages: fetch_node.variable_usages.as_ref(),
                    operation_name: fetch_node.operation_name.as_deref(),
                    operation_kind: fetch_node.operation_kind.as_ref(),
                    operation: &fetch_node.operation,
                    output_rewrites: fetch_node.output_rewrites.as_deref(),
                    raw_variable_values: None,
                    affected_path: None,
                })
                .boxed(),
            ),
            PlanNode::BatchFetch(batch_fetch_node) => {
                let (raw_variable_values, aliases) =
                    self.prepare_batch_fetch_job_state(&batch_fetch_node.entity_batch, data);

                if aliases
                    .iter()
                    .all(|alias| alias.representation_hash_to_index.is_empty())
                {
                    // All alias lists are empty, so nothing to fetch.
                    // We skip the network call to save time.
                    tracing::trace!(
                        alias_count = aliases.len(),
                        "Skipping batched entity fetch with no representations"
                    );
                    return None;
                }

                Some(
                    self.prepare_execution_job(PrepareExecutionJobOpts {
                        subgraph_name: &batch_fetch_node.service_name,
                        variable_usages: batch_fetch_node.variable_usages.as_ref(),
                        operation_name: batch_fetch_node.operation_name.as_deref(),
                        operation_kind: batch_fetch_node.operation_kind.as_ref(),
                        operation: &batch_fetch_node.operation,
                        output_rewrites: None,
                        raw_variable_values: Some(raw_variable_values),
                        affected_path: None,
                    })
                    .map_ok(|fetch_job| ExecutionJob::BatchFetch {
                        subgraph_name: fetch_job.subgraph_name(),
                        response: fetch_job.response(),
                        aliases,
                    })
                    .boxed(),
                )
            }
            PlanNode::Flatten(flatten_node) => {
                let fetch_node = match flatten_node.node.as_ref() {
                    PlanNode::Fetch(fetch_node) => fetch_node,
                    _ => return None,
                };
                let requires_nodes = fetch_node.requires.as_ref()?;

                let mut index = 0;
                let normalized_path = flatten_node.path.as_slice();
                let mut filtered_representations = Vec::new();
                filtered_representations.put(OPEN_BRACKET);
                let possible_types = &self.schema_metadata.possible_types;
                let mut representation_hashes: Vec<u64> = Vec::new();
                let mut representation_hash_to_index: AHashMap<u64, usize> = AHashMap::new();
                let arena = bumpalo::Bump::new();

                traverse_and_callback(
                    data,
                    normalized_path,
                    &self.schema_metadata.possible_types,
                    &mut |entity| {
                        let hash = entity.to_hash(&requires_nodes.items, possible_types);

                        if !entity.is_null() {
                            representation_hashes.push(hash);
                        }

                        let is_first_representation = representation_hash_to_index.is_empty();
                        let vacant_entry = match representation_hash_to_index.entry(hash) {
                            Entry::Occupied(_) => return,
                            Entry::Vacant(vacant_entry) => vacant_entry,
                        };

                        let entity = if let Some(input_rewrites) = &fetch_node.input_rewrites {
                            let new_entity = arena.alloc(entity.clone());
                            for input_rewrite in input_rewrites {
                                input_rewrite
                                    .rewrite(&self.schema_metadata.possible_types, new_entity);
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
                            is_first_representation,
                            None,
                        );

                        if is_projected {
                            vacant_entry.insert(index);
                        }

                        index += 1;
                    },
                );

                filtered_representations.put(CLOSE_BRACKET);

                if representation_hash_to_index.is_empty() {
                    return None;
                }

                // This is the future for the actual fetch job
                Some(
                    self.prepare_execution_job(PrepareExecutionJobOpts {
                        subgraph_name: &fetch_node.service_name,
                        variable_usages: fetch_node.variable_usages.as_ref(),
                        operation_name: fetch_node.operation_name.as_deref(),
                        operation_kind: fetch_node.operation_kind.as_ref(),
                        operation: &fetch_node.operation,
                        output_rewrites: fetch_node.output_rewrites.as_deref(),
                        raw_variable_values: Some(vec![(
                            "representations",
                            filtered_representations,
                        )]),
                        affected_path: Some(&flatten_node.path),
                    })
                    .map_ok(|fetch_job| ExecutionJob::FlattenFetch {
                        flatten_node_path: &flatten_node.path,
                        response: fetch_job.response(),
                        subgraph_name: fetch_node.service_name.as_str(),
                        representation_hashes,
                        representation_hash_to_index,
                        output_rewrites: fetch_node.output_rewrites.as_deref(),
                    })
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

                match job {
                    ExecutionJob::Fetch {
                        mut response,
                        output_rewrites,
                        ..
                    } => {
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

                        ctx.handle_errors(subgraph_name, affected_path, response.errors, None);
                    }
                    ExecutionJob::FlattenFetch {
                        mut response,
                        flatten_node_path,
                        representation_hashes,
                        ref representation_hash_to_index,
                        output_rewrites,
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

                            ctx.handle_errors(
                                subgraph_name,
                                affected_path,
                                response.errors,
                                entity_index_error_map,
                            );
                        } else {
                            ctx.handle_errors(subgraph_name, affected_path, response.errors, None);
                        }
                    }
                    ExecutionJob::BatchFetch {
                        mut response,
                        aliases,
                        ..
                    } => {
                        if let Some(response_bytes) = response.bytes {
                            ctx.response_storage.add_response(response_bytes);
                        }

                        // Split errors by alias
                        let mut errors =
                            self.partition_batch_errors_by_alias(&aliases, response.errors.take());
                        // Take returned entities per alias
                        let mut entities_by_alias =
                            Self::collect_batched_entities_by_alias(&mut response.data, &aliases);

                        for (alias_index, alias_state) in aliases.iter().enumerate() {
                            let alias_index = AliasIndex(alias_index);
                            let mut alias_errors = errors.by_alias_index.remove(&alias_index);
                            // Merge entities back into execution context (final data)
                            // and attach alias errors and unmatched errors
                            let entity_index_error_map = self.merge_batch_alias_entities(
                                ctx,
                                alias_state,
                                entities_by_alias.get_mut(&alias_index),
                                alias_errors.as_deref(),
                            );

                            let affected_path = if alias_state.paths.len() == 1 {
                                Some(alias_state.paths[0].merge_path)
                            } else {
                                None
                            };

                            // Attach alias errors
                            ctx.handle_errors(
                                subgraph_name,
                                affected_path,
                                alias_errors.take(),
                                entity_index_error_map,
                            );
                        }

                        // Attach errors that do not point to any known alias.
                        if !errors.unmatched.is_empty() {
                            ctx.handle_errors(subgraph_name, None, Some(errors.unmatched), None);
                        }

                        tracing::trace!(
                            alias_count = aliases.len(),
                            "Patched entity batch alias results"
                        );
                    }
                }
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

    fn partition_batch_errors_by_alias(
        &self,
        aliases: &[AliasBatchState<'exec>],
        response_errors: Option<Vec<GraphQLError>>,
    ) -> BatchFetchErrors {
        // Split subgraph errors into:
        // - errors that belong to a known alias
        // - errors that do not match any alias
        let mut alias_index_by_name: AHashMap<&str, AliasIndex> =
            AHashMap::with_capacity(aliases.len());
        for (alias_index, alias_state) in aliases.iter().enumerate() {
            alias_index_by_name.insert(
                alias_state.alias_spec.alias.as_str(),
                AliasIndex(alias_index),
            );
        }

        let mut errors_by_alias_index: AHashMap<AliasIndex, Vec<GraphQLError>> = AHashMap::new();
        let mut unmatched_errors: Vec<GraphQLError> = Vec::new();

        let Some(response_errors) = response_errors else {
            return BatchFetchErrors::default();
        };

        for mut error in response_errors {
            let maybe_alias = error.path.as_ref().and_then(|path| {
                path.segments.first().and_then(|segment| match segment {
                    GraphQLErrorPathSegment::String(alias) => Some(alias.as_str()),
                    _ => None,
                })
            });

            let Some(alias) = maybe_alias else {
                unmatched_errors.push(error);
                continue;
            };

            let Some(alias_index) = alias_index_by_name.get(alias) else {
                unmatched_errors.push(error);
                continue;
            };

            if let Some(path) = error.path.as_mut() {
                // Subgraph batch errors use alias names like "_e0".
                // Our error normalizer (GraphQLError::normalize_entity_error)
                // expects paths like ["_entities", index, ...].
                // So we replace the first path segment with "_entities".
                // Before: ["_e0", 2, "price"]
                // After : ["_entities", 2, "price"]
                if let Some(GraphQLErrorPathSegment::String(first)) = path.segments.first_mut() {
                    *first = "_entities".to_string();
                }
            }

            errors_by_alias_index
                .entry(*alias_index)
                .or_default()
                .push(error);
        }

        BatchFetchErrors {
            by_alias_index: errors_by_alias_index,
            unmatched: unmatched_errors,
        }
    }

    fn collect_batched_entities_by_alias(
        response_data: &mut Value<'exec>,
        aliases: &[AliasBatchState<'exec>],
    ) -> AHashMap<AliasIndex, Vec<Value<'exec>>> {
        // Take entity arrays from response data once per alias.
        // This avoids repeated lookups/mutations on response data.
        let mut entities_by_alias: AHashMap<AliasIndex, Vec<Value<'exec>>> =
            AHashMap::with_capacity(aliases.len());

        for (alias_index, alias_state) in aliases.iter().enumerate() {
            let Some(entities) =
                response_data.take_entities_by_key(alias_state.alias_spec.alias.as_str())
            else {
                continue;
            };
            entities_by_alias.insert(AliasIndex(alias_index), entities);
        }

        entities_by_alias
    }

    /// Merge one alias's returned entities back into `ctx.data`.
    fn merge_batch_alias_entities<'alias>(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        alias_state: &'alias AliasBatchState<'exec>,
        entities: Option<&mut Vec<Value<'exec>>>,
        alias_errors: Option<&[GraphQLError]>,
    ) -> Option<HashMap<&'alias usize, Vec<GraphQLErrorPath>>> {
        let has_alias_errors = alias_errors.is_some();
        let mut entity_index_error_map = has_alias_errors.then(HashMap::new);
        let Some(entities) = entities else {
            return entity_index_error_map;
        };

        if let Some(output_rewrites) = alias_state.alias_spec.output_rewrites.as_ref() {
            for output_rewrite in output_rewrites {
                for entity in entities.iter_mut() {
                    output_rewrite.rewrite(&self.schema_metadata.possible_types, entity);
                }
            }
        }

        if alias_state.representation_hash_to_index.is_empty() {
            return entity_index_error_map;
        }

        // We walk each merge path
        for path_state in &alias_state.paths {
            let mut index = 0;
            let normalized_path = path_state.merge_path.as_slice();
            let initial_error_path = has_alias_errors
                // Small extra capacity for path segments that will be appended later.
                .then(|| GraphQLErrorPath::with_capacity(normalized_path.len() + 2));

            // For each visited target:
            traverse_and_callback_mut(
                &mut ctx.data,
                normalized_path,
                self.schema_metadata,
                initial_error_path,
                &mut |target_data, error_path| {
                    let hash = path_state.representation_hashes[index];
                    // Find matching entity index from hash->index map
                    if let Some(entity_index) = alias_state.representation_hash_to_index.get(&hash)
                    {
                        // If this alias has errors, we also collect target paths
                        // so one subgraph error can be copied to all matching targets.
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
                            // The transmute is to satisfy the compiler, but the lifetime is valid.
                            let new_val: Value<'_> = unsafe { std::mem::transmute(entity.clone()) };
                            deep_merge(target_data, new_val);
                        }
                    }

                    index += 1;
                },
            );
        }

        entity_index_error_map
    }

    // The preperation includes:
    // - building one `_entities` input list for each alias
    // - remembering where each item came from (so we can put results back)
    fn prepare_batch_fetch_job_state(
        &self,
        entity_batch: &'exec EntityBatch,
        data: &Value<'exec>,
    ) -> (Vec<(&'exec str, Vec<u8>)>, Vec<AliasBatchState<'exec>>) {
        let mut raw_variable_values: Vec<(&'exec str, Vec<u8>)> =
            Vec::with_capacity(entity_batch.aliases.len());
        let mut raw_variable_indices_by_name: AHashMap<&'exec str, usize> =
            AHashMap::with_capacity(entity_batch.aliases.len());
        let mut aliases = Vec::with_capacity(entity_batch.aliases.len());

        let possible_types = &self.schema_metadata.possible_types;

        for alias_spec in &entity_batch.aliases {
            let mut index = 0;
            let mut filtered_representations = Vec::new();
            filtered_representations.put(OPEN_BRACKET);
            let mut representation_hash_to_index: AHashMap<u64, usize> = AHashMap::new();
            let arena = bumpalo::Bump::new();
            let mut path_hashes_by_index: Vec<Option<Arc<Vec<u64>>>> =
                vec![None; alias_spec.merge_paths.len()];

            let mut path_groups: Vec<(&FlattenNodePath, Vec<usize>)> =
                Vec::with_capacity(alias_spec.merge_paths.len());
            for (path_index, merge_path) in alias_spec.merge_paths.iter().enumerate() {
                if let Some((_, target_indices)) =
                    path_groups.iter_mut().find(|(path, _)| *path == merge_path)
                {
                    target_indices.push(path_index);
                } else {
                    path_groups.push((merge_path, vec![path_index]));
                }
            }

            for (merge_path, grouped_target_indices) in path_groups {
                let mut representation_hashes: Vec<u64> = Vec::new();

                traverse_and_callback(data, merge_path.as_slice(), possible_types, &mut |entity| {
                    let hash = entity.to_hash(&alias_spec.requires.items, possible_types);

                    if !entity.is_null() {
                        representation_hashes.push(hash);
                    }

                    let is_first_representation = representation_hash_to_index.is_empty();
                    let vacant_entry = match representation_hash_to_index.entry(hash) {
                        Entry::Occupied(_) => return,
                        Entry::Vacant(vacant_entry) => vacant_entry,
                    };

                    let entity = if let Some(input_rewrites) = &alias_spec.input_rewrites {
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
                        &alias_spec.requires.items,
                        entity,
                        &mut filtered_representations,
                        is_first_representation,
                        None,
                    );

                    if is_projected {
                        vacant_entry.insert(index);
                    }

                    index += 1;
                });

                let representation_hashes = Arc::new(representation_hashes);

                for path_index in grouped_target_indices {
                    path_hashes_by_index[path_index] = Some(Arc::clone(&representation_hashes));
                }
            }

            filtered_representations.put(CLOSE_BRACKET);

            let mut paths = Vec::with_capacity(alias_spec.merge_paths.len());
            for (path_index, merge_path) in alias_spec.merge_paths.iter().enumerate() {
                paths.push(AliasPathState {
                    merge_path,
                    representation_hashes: path_hashes_by_index[path_index]
                        .take()
                        .unwrap_or_else(|| Arc::new(Vec::new())),
                });
            }

            let variable_name = alias_spec.representations_variable_name.as_str();
            if !raw_variable_indices_by_name.contains_key(variable_name) {
                raw_variable_indices_by_name.insert(variable_name, raw_variable_values.len());
                raw_variable_values.push((variable_name, filtered_representations));
            }

            aliases.push(AliasBatchState {
                alias_spec,
                representation_hash_to_index,
                paths,
            });
        }

        (raw_variable_values, aliases)
    }

    async fn prepare_execution_job(
        &self,
        opts: PrepareExecutionJobOpts<'exec>,
    ) -> Result<ExecutionJob<'exec>, PlanExecutionError> {
        let subgraph_operation_span =
            GraphQLSubgraphOperationSpan::new(opts.subgraph_name, &opts.operation.document_str);

        async {
            // TODO: We could optimize header map creation by caching them per service name
            let mut headers_map = HeaderMap::new();
            let subgraph_name_factory = || Some(opts.subgraph_name.to_string());
            let affected_path_factory = || opts.affected_path.map(|p| p.to_string());
            modify_subgraph_request_headers(
                self.headers_plan,
                opts.subgraph_name,
                self.client_request,
                &mut headers_map,
            )
            .with_plan_context(LazyPlanContext {
                subgraph_name: subgraph_name_factory,
                affected_path: affected_path_factory,
            })?;
            let variable_refs = select_fetch_variables(self.variable_values, opts.variable_usages);

            let mut subgraph_request = SubgraphExecutionRequest {
                query: &opts.operation.document_str,
                dedupe: self.dedupe_subgraph_requests,
                operation_name: opts.operation_name,
                variables: variable_refs,
                raw_variable_values: opts.raw_variable_values,
                headers: headers_map,
                extensions: None,
            };

            let client_document_hash_str = opts.operation.hash.to_string();
            subgraph_operation_span.record_operation_identity(GraphQLSpanOperationIdentity {
                name: opts.operation_name,
                operation_type: match opts.operation_kind {
                    Some(OperationKind::Query) | None => "query",
                    Some(OperationKind::Mutation) => "mutation",
                    Some(OperationKind::Subscription) => "subscription",
                },
                client_document_hash: &client_document_hash_str,
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
                    opts.subgraph_name,
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
                subgraph_name: opts.subgraph_name,
                response,
                output_rewrites: opts.output_rewrites,
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
        response::graphql_error::{GraphQLErrorExtensions, GraphQLErrorPath},
        response::value::Value as ResponseValue,
        SubgraphExecutorMap,
    };

    use super::select_fetch_variables;
    use graphql_tools::parser::query;
    use hive_router_config::HiveRouterConfig;
    use hive_router_internal::telemetry::TelemetryContext;
    use hive_router_query_planner::{
        ast::{
            document::Document,
            operation::{OperationDefinition, SubgraphFetchOperation},
            selection_set::SelectionSet,
        },
        planner::plan_nodes::{EntityBatch, EntityBatchAlias, FetchNode, ParallelNode, PlanNode},
        utils::parsing::parse_operation,
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

    #[test]
    fn prepare_batch_fetch_job_state_deduplicates_shared_variable_payloads() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let subgraph_endpoint_map = HashMap::from([(
            "inventory".to_string(),
            "http://example.com/graphql".parse().unwrap(),
        )]);

        let executors = SubgraphExecutorMap::from_http_endpoint_map(
            &subgraph_endpoint_map,
            HiveRouterConfig::default().into(),
            Arc::new(TelemetryContext::from_propagation_config(
                &Default::default(),
            )),
        )
        .unwrap();

        let executor = Executor {
            variable_values: &None,
            schema_metadata: &SchemaMetadata::default(),
            executors: &executors,
            client_request: &ClientRequestDetails {
                method: &http::Method::POST,
                url: &"http://example.com".parse().unwrap(),
                headers: &HeaderMap::new(),
                operation: OperationDetails {
                    name: None,
                    query: "{ products { upc } }",
                    kind: "query",
                },
                jwt: JwtRequestDetails::Unauthenticated,
            },
            headers_plan: &HeaderRulesPlan::default(),
            jwt_forwarding_plan: None,
            dedupe_subgraph_requests: false,
            plugin_req_state: &None,
        };

        let data: ResponseValue = sonic_rs::from_str(
            r#"{
                "products": [
                    {"__typename": "Product", "upc": "1"},
                    {"__typename": "Product", "upc": "2"}
                ]
            }"#,
        )
        .unwrap();

        fn document_into_selection<'a>(
            doc: query::Document<'a, String>,
        ) -> query::SelectionSet<'a, String> {
            doc.definitions
                .iter()
                .find_map(|def| {
                    let query::Definition::Operation(op) = def else {
                        return None;
                    };
                    match op {
                        query::OperationDefinition::SelectionSet(sel) => Some(sel),
                        query::OperationDefinition::Query(q) => Some(&q.selection_set),
                        query::OperationDefinition::Mutation(m) => Some(&m.selection_set),
                        query::OperationDefinition::Subscription(s) => Some(&s.selection_set),
                    }
                })
                .unwrap()
                .clone()
        }

        let requires_query = parse_operation("{ ... on Product { upc } }");
        let requires_selection = document_into_selection(requires_query);

        let shared_var = "__batch_reps_0".to_string();
        let entity_batch = EntityBatch {
            aliases: vec![
                EntityBatchAlias {
                    alias: "_e0".to_string(),
                    representations_variable_name: shared_var.clone(),
                    merge_paths: vec![],
                    requires: requires_selection.clone().into(),
                    input_rewrites: None,
                    output_rewrites: None,
                },
                EntityBatchAlias {
                    alias: "_e1".to_string(),
                    representations_variable_name: shared_var,
                    merge_paths: vec![],
                    requires: requires_selection.into(),
                    input_rewrites: None,
                    output_rewrites: None,
                },
            ],
        };

        let (raw_variable_values, aliases) =
            executor.prepare_batch_fetch_job_state(&entity_batch, &data);

        assert_eq!(aliases.len(), 2);
        assert_eq!(raw_variable_values.len(), 1);
        assert_eq!(raw_variable_values[0].0, "__batch_reps_0");
    }

    #[tokio::test]
    async fn runs_parallel_jobs_in_parallel() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut subgraph_a = mockito::Server::new_async().await;
        let mut subgraph_b = mockito::Server::new_async().await;
        let data = crate::response::value::Value::Null;
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
        let executor = Executor {
            variable_values: &None,
            schema_metadata: &SchemaMetadata::default(),
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
        let data_ref: &'static crate::response::value::Value<'static> =
            unsafe { std::mem::transmute(&exec_ctx.data) };

        let (sender, receiver) = channel();

        let mock_b = subgraph_b
            .mock("POST", "/graphql")
            .with_chunked_body(move |writer| {
                // We can add some delay here to make sure the parallel execution is actually working
                std::thread::sleep(Duration::from_millis(1000));
                // data should have `from_a` field from subgraph_a's response,
                // so data the merging process does not wait for subgraph_b's response to merge subgraph_a's response
                if let Some(data) = data_ref.as_object() {
                    let from_a_index = data.iter().position(|(k, _)| k == &"from_a");
                    let from_a_value = from_a_index
                        .and_then(|index| data.get(index))
                        .and_then(|(_, v)| v.as_str());
                    if let Some(from_a_value) = from_a_value {
                        sender
                            .send(from_a_value.to_string())
                            .expect("Failed to send from_a value through channel");
                    }
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
