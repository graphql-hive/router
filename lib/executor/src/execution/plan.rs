use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use bytes::{BufMut, Bytes};
use futures::{future::BoxFuture, stream::BoxStream, stream::FuturesUnordered, StreamExt};
use hive_router_query_planner::planner::plan_nodes::{
    ConditionNode, FetchNode, FetchRewrite, FlattenNode, FlattenNodePath, ParallelNode, PlanNode,
    QueryPlan, SequenceNode,
};
use http::HeaderMap;
use ntex::http::HeaderMap as NtexHeaderMap;
use serde::Deserialize;
use sonic_rs::ValueRef;

use crate::{
    context::ExecutionContext,
    execution::{
        client_request_details::ClientRequestDetails,
        error::{IntoPlanExecutionError, LazyPlanContext, PlanExecutionError},
        jwt_forward::JwtAuthForwardingPlan,
        rewrites::FetchRewriteExt,
    },
    executors::{
        common::{HttpExecutionResponse, SubgraphExecutionRequest},
        map::SubgraphExecutorMap,
    },
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
        graphql_error::{GraphQLError, GraphQLErrorExtensions, GraphQLErrorPath},
        merge::deep_merge,
        subgraph_response::SubgraphResponse,
        value::Value,
    },
    utils::{
        consts::{CLOSE_BRACKET, OPEN_BRACKET},
        traverse::{traverse_and_callback, traverse_and_callback_mut},
    },
};

pub struct QueryPlanExecutionContext<'exec, 'req> {
    pub query_plan: &'exec QueryPlan,
    /// Projection plan - Arc because subscriptions need to own it
    pub projection_plan: Arc<Vec<FieldProjectionPlan>>,
    /// Headers plan - Arc because subscriptions need to own it
    pub headers_plan: Arc<HeaderRulesPlan>,
    pub variable_values: Option<HashMap<String, sonic_rs::Value>>,
    pub extensions: Option<HashMap<String, sonic_rs::Value>>,
    pub client_request: &'exec ClientRequestDetails<'exec, 'req>,
    pub introspection_context: &'exec IntrospectionContext<'exec, 'static>,
    pub operation_type_name: &'exec str,
    /// Executors - Arc because subscriptions need to own it
    pub executors: Arc<SubgraphExecutorMap>,
    pub jwt_auth_forwarding: &'exec Option<JwtAuthForwardingPlan>,
    pub initial_errors: Vec<GraphQLError>,
    /// Schema metadata - Arc because subscriptions need to own it
    pub schema_metadata: Arc<SchemaMetadata>,
}

pub struct PlanExecutionOutput {
    pub body: Vec<u8>,
    pub headers: HeaderMap,
    pub error_count: usize,
}

pub struct PlanSubscriptionOutput {
    pub body: BoxStream<'static, Vec<u8>>,
    pub headers: HeaderMap,
    pub error_count: usize,
}

/// Result of executing a query plan - either a single response or a subscription stream
pub enum QueryPlanExecutionResult {
    Single(PlanExecutionOutput),
    Stream(PlanSubscriptionOutput),
}

/// Owned context for subscription stream processing.
/// Contains all data needed to execute entity resolution for each subscription event.
/// Derived from QueryPlanExecutionContext by cloning Arc fields and owning String data.
struct OwnedQueryPlanExecutionContext {
    /// The query plan for entity resolution (remaining nodes after subscription)
    query_plan: Arc<QueryPlan>,
    projection_plan: Arc<Vec<FieldProjectionPlan>>,
    headers_plan: Arc<HeaderRulesPlan>,
    variable_values: Option<HashMap<String, sonic_rs::Value>>,
    extensions: Option<HashMap<String, sonic_rs::Value>>,
    operation_type_name: String,
    executors: Arc<SubgraphExecutorMap>,
    schema_metadata: Arc<SchemaMetadata>,
    initial_errors: Vec<GraphQLError>,
}

impl OwnedQueryPlanExecutionContext {
    /// Create owned context from QueryPlanExecutionContext for subscription processing.
    /// The `remaining_plan_nodes` become the query plan for entity resolution.
    fn from_ctx(
        ctx: &QueryPlanExecutionContext<'_, '_>,
        remaining_plan_nodes: Option<Vec<PlanNode>>,
    ) -> Self {
        // Create a synthetic query plan for entity resolution from remaining nodes
        let query_plan = Arc::new(QueryPlan {
            kind: "QueryPlan".to_string(),
            node: remaining_plan_nodes.map(|nodes| {
                if nodes.len() == 1 {
                    nodes.into_iter().next().unwrap()
                } else {
                    PlanNode::Sequence(SequenceNode { nodes })
                }
            }),
        });

        Self {
            query_plan,
            projection_plan: ctx.projection_plan.clone(),
            headers_plan: ctx.headers_plan.clone(),
            variable_values: ctx.variable_values.clone(),
            extensions: ctx.extensions.clone(),
            operation_type_name: ctx.operation_type_name.to_string(),
            executors: ctx.executors.clone(),
            schema_metadata: ctx.schema_metadata.clone(),
            initial_errors: ctx.initial_errors.clone(),
        }
    }
}

pub async fn execute_query_plan<'exec, 'req>(
    ctx: QueryPlanExecutionContext<'exec, 'req>,
) -> Result<QueryPlanExecutionResult, PlanExecutionError> {
    let (subscription_node, remaining_nodes) = match &ctx.query_plan.node {
        // a subscription to a subgraph that contains all data and doesn't need entity resolution
        Some(PlanNode::Subscription(sub)) => (Some(sub), None),
        // a subscription that needs entity resolution. after emitting, it needs to execute the
        // remaining plan nodes in the sequence
        Some(PlanNode::Sequence(seq)) => match seq.nodes.first() {
            Some(PlanNode::Subscription(sub)) => {
                let remaining = if seq.nodes.len() > 1 {
                    Some(seq.nodes[1..].to_vec())
                } else {
                    None
                };
                (Some(sub), remaining)
            }
            _ => (None, None),
        },
        _ => (None, None),
    };

    if let Some(sub) = subscription_node {
        // subscription

        let fetch_node = sub.primary.clone();

        let variables = ctx.variable_values.as_ref().map(|vars| {
            vars.iter()
                .map(|(k, v)| (k.as_str(), v))
                .collect::<HashMap<_, _>>()
        });

        let request = SubgraphExecutionRequest {
            query: fetch_node.operation.document_str.as_str(),
            dedupe: false,
            operation_name: fetch_node.operation_name.as_deref(),
            variables,
            representations: None,
            headers: HeaderMap::new(),
            extensions: None,
        };

        let response_stream = ctx
            .executors
            .subscribe(&fetch_node.service_name, request, ctx.client_request)
            .await;

        // subscription needs owned context for the stream
        let owned_ctx = Box::new(OwnedQueryPlanExecutionContext::from_ctx(
            &ctx,
            remaining_nodes,
        ));

        // Create a stream of serialized subscription events
        let body_stream = Box::pin(async_stream::stream! {
            use crate::execution::client_request_details::{
                ClientRequestDetails, JwtRequestDetails, OperationDetails,
            };

            let mut response_stream = response_stream;

            while let Some(response) = response_stream.next().await {
                let response_body = &response.body;

                // Parse the subgraph response
                let subgraph_response: Result<SubgraphResponse, _> = sonic_rs::from_slice(response_body);

                let output = match subgraph_response {
                    Ok(parsed_response) => {
                        let initial_data = parsed_response.data;
                        let mut initial_errors: Vec<GraphQLError> = owned_ctx.initial_errors.clone();
                        if let Some(resp_errors) = parsed_response.errors {
                            initial_errors.extend(resp_errors);
                        }

                        if owned_ctx.query_plan.node.is_some() {
                            // entity resolution

                            // Build a dummy client request details for entity resolution
                            // TODO: it cant be a dummy, it needs to make sense
                            let method = http::Method::POST;
                            let uri: http::Uri = "/graphql".parse().unwrap();
                            let headers = NtexHeaderMap::new();
                            let jwt = JwtRequestDetails::Unauthenticated;
                            let operation = OperationDetails {
                                name: None,
                                query: "",
                                kind: "subscription",
                            };
                            let client_request = ClientRequestDetails {
                                method: &method,
                                url: &uri,
                                headers: &headers,
                                operation,
                                jwt: &jwt,
                            };

                            match execute_plan_with_initial_data(
                                &owned_ctx.query_plan,
                                &owned_ctx.projection_plan,
                                &owned_ctx.headers_plan,
                                &owned_ctx.variable_values,
                                &owned_ctx.extensions,
                                &owned_ctx.operation_type_name,
                                &owned_ctx.executors,
                                &owned_ctx.schema_metadata,
                                &client_request,
                                &None, // no jwt forwarding for entity resolution. TODO: or?
                                initial_data,
                                initial_errors,
                                response_body.len() + 256,
                            )
                            .await
                            {
                                Ok(output) => output.body,
                                Err(e) => {
                                    // TODO: error details leaking? should this be masked?
                                    let error = GraphQLError::from_message_and_extensions(
                                        format!("Entity resolution error: {}", e),
                                        Default::default(),
                                    );
                                    format_error_response(&[error])
                                }
                            }
                        } else {
                            // no entity resolution, just project the response
                            match project_by_operation(
                                &initial_data,
                                initial_errors,
                                &owned_ctx.extensions,
                                &owned_ctx.operation_type_name,
                                &owned_ctx.projection_plan,
                                &owned_ctx.variable_values,
                                response_body.len() + 256,
                            ) {
                                Ok(body) => body,
                                Err(e) => {
                                    let error = GraphQLError::from_message_and_extensions(
                                        format!("Projection error: {}", e),
                                        Default::default(),
                                    );
                                    format_error_response(&[error])
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let error = GraphQLError::from_message_and_extensions(
                            format!("Failed to parse subgraph response: {}", e),
                            Default::default(),
                        );
                        format_error_response(&[error])
                    }
                };

                yield output;
            }
        });

        // TODO: Extract headers from first response or aggregate them
        let headers = HeaderMap::new();

        return Ok(QueryPlanExecutionResult::Stream(PlanSubscriptionOutput {
            body: body_stream,
            headers,
            error_count: 0, // TODO: Track errors across subscription events
        }));
    }

    // query/mutation

    let init_value = if let Some(introspection_query) = ctx.introspection_context.query {
        resolve_introspection(introspection_query, ctx.introspection_context)
    } else if ctx.projection_plan.is_empty() {
        Value::Null
    } else {
        Value::Object(Vec::new())
    };

    execute_plan_with_initial_data(
        ctx.query_plan,
        &ctx.projection_plan,
        &ctx.headers_plan,
        &ctx.variable_values,
        &ctx.extensions,
        ctx.operation_type_name,
        &ctx.executors,
        ctx.introspection_context.metadata,
        ctx.client_request,
        ctx.jwt_auth_forwarding,
        init_value,
        ctx.initial_errors,
        0, // Size estimated from response storage
    )
    .await
    .map(QueryPlanExecutionResult::Single)
}

#[allow(clippy::too_many_arguments)]
/// Core execution logic shared between regular plan execution and subscription event processing.
/// Executes the plan nodes and projects the final response.
async fn execute_plan_with_initial_data<'exec, 'req>(
    query_plan: &'exec QueryPlan,
    projection_plan: &Arc<Vec<FieldProjectionPlan>>,
    headers_plan: &'exec HeaderRulesPlan,
    variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    extensions: &Option<HashMap<String, sonic_rs::Value>>,
    operation_type_name: &str,
    executors: &'exec SubgraphExecutorMap,
    schema_metadata: &'exec SchemaMetadata,
    client_request: &'exec ClientRequestDetails<'exec, 'req>,
    jwt_auth_forwarding: &'exec Option<JwtAuthForwardingPlan>,
    initial_data: Value<'_>,
    initial_errors: Vec<GraphQLError>,
    estimated_response_size: usize,
) -> Result<PlanExecutionOutput, PlanExecutionError> {
    // Clone initial_data to make it 'static for ExecutionContext
    // TODO: is this wasteful if initial data is null?
    // SAFETY: We're creating a new owned value that will be used within this function
    let owned_data: Value<'exec> = unsafe { std::mem::transmute(initial_data.clone()) };

    let mut exec_ctx = ExecutionContext::new(query_plan, owned_data, initial_errors);

    let executor = Executor::new(
        variable_values,
        executors,
        schema_metadata,
        client_request,
        headers_plan,
        jwt_auth_forwarding,
        // Subscriptions never dedupe (operation_type_name != "Query")
        operation_type_name == "Query",
    );

    if query_plan.node.is_some() {
        executor
            .execute(&mut exec_ctx, query_plan.node.as_ref())
            .await?;
    }

    let mut response_headers = HeaderMap::new();
    modify_client_response_headers(exec_ctx.response_headers_aggregator, &mut response_headers)
        .with_plan_context(LazyPlanContext {
            subgraph_name: || None,
            affected_path: || None,
        })?;

    let final_response = &exec_ctx.final_response;
    let error_count = exec_ctx.errors.len();
    let body = project_by_operation(
        final_response,
        exec_ctx.errors,
        extensions,
        operation_type_name,
        projection_plan,
        variable_values,
        estimated_response_size.max(exec_ctx.response_storage.estimate_final_response_size()),
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

/// Format an error response as JSON bytes
fn format_error_response(errors: &[GraphQLError]) -> Vec<u8> {
    let errors_json: Vec<String> = errors
        .iter()
        .map(|e| format!(r#"{{"message":"{}"}}"#, e.message.replace('"', "\\\"")))
        .collect();

    format!(r#"{{"data":null,"errors":[{}]}}"#, errors_json.join(",")).into_bytes()
}

pub struct Executor<'exec, 'req> {
    variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    schema_metadata: &'exec SchemaMetadata,
    executors: &'exec SubgraphExecutorMap,
    client_request: &'exec ClientRequestDetails<'exec, 'req>,
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

struct SubgraphOutput {
    body: Bytes,
    headers: HeaderMap,
}

struct FetchJob {
    fetch_node_id: i64,
    subgraph_name: String,
    response: SubgraphOutput,
}

struct FlattenFetchJob {
    flatten_node_path: FlattenNodePath,
    response: SubgraphOutput,
    fetch_node_id: i64,
    subgraph_name: String,
    representation_hashes: Vec<u64>,
    representation_hash_to_index: HashMap<u64, usize>,
}

enum ExecutionJob {
    Fetch(FetchJob),
    FlattenFetch(FlattenFetchJob),
    None,
}

impl From<ExecutionJob> for SubgraphOutput {
    fn from(value: ExecutionJob) -> Self {
        match value {
            ExecutionJob::Fetch(j) => Self {
                body: j.response.body,
                headers: j.response.headers,
            },
            ExecutionJob::FlattenFetch(j) => Self {
                body: j.response.body,
                headers: j.response.headers,
            },
            ExecutionJob::None => Self {
                body: Bytes::new(),
                headers: HeaderMap::new(),
            },
        }
    }
}

impl From<HttpExecutionResponse> for SubgraphOutput {
    fn from(res: HttpExecutionResponse) -> Self {
        Self {
            body: res.body,
            headers: res.headers,
        }
    }
}

struct PreparedFlattenData {
    representations: Vec<u8>,
    representation_hashes: Vec<u64>,
    representation_hash_to_index: HashMap<u64, usize>,
}

impl<'exec, 'req> Executor<'exec, 'req> {
    pub fn new(
        variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
        executors: &'exec SubgraphExecutorMap,
        schema_metadata: &'exec SchemaMetadata,
        client_request: &'exec ClientRequestDetails<'exec, 'req>,
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

    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        plan: Option<&PlanNode>,
    ) -> Result<(), PlanExecutionError> {
        match plan {
            Some(PlanNode::Fetch(node)) => self.execute_fetch_wave(ctx, node).await,
            Some(PlanNode::Parallel(node)) => self.execute_parallel_wave(ctx, node).await,
            Some(PlanNode::Sequence(node)) => self.execute_sequence_wave(ctx, node).await,
            // When doing subscription entity resolution, we execute a query plan with the
            // remainder of the nodes after the SubscriptionNode. In that case, the root
            // can be a Flatten node and we need to start executing from there.
            // NOTE: this wont work `Some(PlanNode::Flatten(node)) => self.execute_plan_node(ctx, &node.node).await`
            // TODO: is there a nicer way to do this?
            Some(PlanNode::Flatten(_)) => self.execute_plan_node(ctx, plan.unwrap()).await,
            // Plans produced by our Query Planner can only start with: Fetch, Sequence or Parallel.
            // Any other node type at the root is not supported, do nothing
            Some(_) => Ok(()),
            // An empty plan is valid, just do nothing
            None => Ok(()),
        }
    }

    async fn execute_fetch_wave(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        node: &FetchNode,
    ) -> Result<(), PlanExecutionError> {
        match self.execute_fetch_node(node, None).await {
            Ok(result) => self.process_job_result(ctx, result),
            Err(err) => {
                self.log_error(&err);
                ctx.errors.push(err.into());
                Ok(())
            }
        }
    }

    async fn execute_sequence_wave(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        node: &SequenceNode,
    ) -> Result<(), PlanExecutionError> {
        for child in &node.nodes {
            Box::pin(self.execute_plan_node(ctx, child)).await?;
        }

        Ok(())
    }

    async fn execute_parallel_wave(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        node: &ParallelNode,
    ) -> Result<(), PlanExecutionError> {
        let mut scope = ConcurrencyScope::new();

        for child in &node.nodes {
            let job_future = self.prepare_job_future(child, &ctx.final_response);
            scope.spawn(job_future);
        }

        let results = scope.join_all().await;

        for result in results {
            match result {
                Ok(job) => {
                    self.process_job_result(ctx, job)?;
                }
                Err(err) => {
                    self.log_error(&err);
                    ctx.errors.push(err.into())
                }
            }
        }

        Ok(())
    }

    async fn execute_plan_node(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        node: &PlanNode,
    ) -> Result<(), PlanExecutionError> {
        match node {
            PlanNode::Fetch(fetch_node) => match self.execute_fetch_node(fetch_node, None).await {
                Ok(job) => {
                    self.process_job_result(ctx, job)?;
                }
                Err(err) => {
                    self.log_error(&err);
                    ctx.errors.push(err.into());
                }
            },
            PlanNode::Parallel(parallel_node) => {
                self.execute_parallel_wave(ctx, parallel_node).await?;
            }
            PlanNode::Flatten(flatten_node) => {
                match self.prepare_flatten_data(&ctx.final_response, flatten_node) {
                    Ok(Some(p)) => {
                        match self
                            .execute_flatten_fetch_node(
                                flatten_node,
                                Some(p.representations),
                                Some(p.representation_hashes),
                                Some(p.representation_hash_to_index),
                            )
                            .await
                        {
                            Ok(job) => {
                                self.process_job_result(ctx, job)?;
                            }
                            Err(err) => {
                                self.log_error(&err);
                                ctx.errors.push(err.into());
                            }
                        }
                    }
                    Ok(None) => { /* do nothing */ }
                    Err(err) => {
                        self.log_error(&err);
                        ctx.errors.push(err.into());
                    }
                }
            }
            PlanNode::Sequence(sequence_node) => {
                self.execute_sequence_wave(ctx, sequence_node).await?;
            }
            PlanNode::Condition(condition_node) => {
                if let Some(node) =
                    condition_node_by_variables(condition_node, self.variable_values)
                {
                    Box::pin(self.execute_plan_node(ctx, node)).await?;
                }
            }
            // An unsupported plan node was found, do nothing.
            _ => {}
        }

        Ok(())
    }

    fn prepare_job_future<'wave>(
        &'wave self,
        node: &'wave PlanNode,
        final_response: &Value<'exec>,
    ) -> BoxFuture<'wave, Result<ExecutionJob, PlanExecutionError>> {
        match node {
            PlanNode::Fetch(fetch_node) => Box::pin(self.execute_fetch_node(fetch_node, None)),
            PlanNode::Flatten(flatten_node) => {
                match self.prepare_flatten_data(final_response, flatten_node) {
                    Ok(Some(p)) => Box::pin(self.execute_flatten_fetch_node(
                        flatten_node,
                        Some(p.representations),
                        Some(p.representation_hashes),
                        Some(p.representation_hash_to_index),
                    )),
                    Ok(None) => Box::pin(async { Ok(ExecutionJob::None) }),
                    Err(e) => Box::pin(async move { Err(e) }),
                }
            }
            PlanNode::Condition(node) => {
                match condition_node_by_variables(node, self.variable_values) {
                    Some(node) => Box::pin(self.prepare_job_future(node, final_response)), // This is already clean.
                    None => Box::pin(async { Ok(ExecutionJob::None) }),
                }
            }
            // Our Query Planner does not produce any other plan node types in ParallelNode
            _ => Box::pin(async { Ok(ExecutionJob::None) }),
        }
    }

    fn process_subgraph_response(
        &self,
        subgraph_name: &str,
        ctx: &mut ExecutionContext<'exec>,
        response_bytes: Bytes,
        fetch_node_id: i64,
    ) -> Option<(SubgraphResponse<'exec>, Option<&'exec Vec<FetchRewrite>>)> {
        let idx = ctx.response_storage.add_response(response_bytes);
        // SAFETY: The `bytes` are transmuted to the lifetime `'a` of the `ExecutionContext`.
        // This is safe because the `response_storage` is part of the `ExecutionContext` (`ctx`)
        // and will live as long as `'a`. The `Bytes` are stored in an `Arc`, so they won't be
        // dropped until all references are gone. The `Value`s deserialized from this byte
        // slice will borrow from it, and they are stored in `ctx.final_response`, which also
        // lives for `'a`.
        let bytes: &'exec [u8] =
            unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };

        // SAFETY: The `output_rewrites` are transmuted to the lifetime `'a`. This is safe
        // because `output_rewrites` is part of `OutputRewritesStorage` which is owned by
        // `ExecutionContext` and lives for `'a`.
        let output_rewrites: Option<&'exec Vec<FetchRewrite>> =
            unsafe { std::mem::transmute(ctx.output_rewrites.get(fetch_node_id)) };

        let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
        let response = match SubgraphResponse::deserialize(&mut deserializer) {
            Ok(response) => response,
            Err(e) => {
                let message = format!("Failed to deserialize subgraph response: {}", e);
                let extensions = GraphQLErrorExtensions::new_from_code_and_service_name(
                    "SUBGRAPH_RESPONSE_DESERIALIZATION_FAILED",
                    subgraph_name,
                );
                let error = GraphQLError::from_message_and_extensions(message, extensions);

                ctx.errors.push(error);
                return None;
            }
        };

        Some((response, output_rewrites))
    }

    fn process_job_result(
        &self,
        ctx: &mut ExecutionContext<'exec>,
        job: ExecutionJob,
    ) -> Result<(), PlanExecutionError> {
        let _: () = match job {
            ExecutionJob::Fetch(job) => {
                apply_subgraph_response_headers(
                    self.headers_plan,
                    &job.subgraph_name,
                    &job.response.headers,
                    self.client_request,
                    &mut ctx.response_headers_aggregator,
                )
                .with_plan_context(LazyPlanContext {
                    subgraph_name: || Some(job.subgraph_name.clone()),
                    affected_path: || None,
                })?;

                if let Some((mut response, output_rewrites)) = self.process_subgraph_response(
                    job.subgraph_name.as_ref(),
                    ctx,
                    job.response.body,
                    job.fetch_node_id,
                ) {
                    ctx.handle_errors(&job.subgraph_name, None, response.errors, None);
                    if let Some(output_rewrites) = output_rewrites {
                        for output_rewrite in output_rewrites {
                            output_rewrite
                                .rewrite(&self.schema_metadata.possible_types, &mut response.data);
                        }
                    }

                    deep_merge(&mut ctx.final_response, response.data);
                }
            }
            ExecutionJob::FlattenFetch(job) => {
                apply_subgraph_response_headers(
                    self.headers_plan,
                    &job.subgraph_name,
                    &job.response.headers,
                    self.client_request,
                    &mut ctx.response_headers_aggregator,
                )
                .with_plan_context(LazyPlanContext {
                    subgraph_name: || Some(job.subgraph_name.clone()),
                    affected_path: || Some(job.flatten_node_path.to_string()),
                })?;

                if let Some((mut response, output_rewrites)) = self.process_subgraph_response(
                    &job.subgraph_name,
                    ctx,
                    job.response.body,
                    job.fetch_node_id,
                ) {
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
                        let normalized_path = job.flatten_node_path.as_slice();
                        // If there is an error in the response, then collect the paths for normalizing the error
                        let initial_error_path = response
                            .errors
                            .as_ref()
                            .map(|_| GraphQLErrorPath::with_capacity(normalized_path.len() + 2));
                        let mut entity_index_error_map = response
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
                                if let Some(entity_index) =
                                    job.representation_hash_to_index.get(&hash)
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
                            &job.subgraph_name,
                            Some(job.flatten_node_path.to_string()),
                            response.errors,
                            entity_index_error_map,
                        );
                    } else if let Some(errors) = response.errors {
                        // No entities were returned, but there are errors to handle.
                        // We associate them with the flattened path and subgraph.
                        let affected_path = job.flatten_node_path.to_string();
                        ctx.errors.extend(errors.into_iter().map(|e| {
                            e.add_subgraph_name(&job.subgraph_name)
                                .add_affected_path(affected_path.clone())
                        }));
                    }
                }
            }
            ExecutionJob::None => {
                // nothing to do
            }
        };
        Ok(())
    }

    fn prepare_flatten_data(
        &self,
        final_response: &Value<'exec>,
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

        Ok(Some(PreparedFlattenData {
            representations: filtered_representations,
            representation_hashes,
            representation_hash_to_index: filtered_representations_hashes,
        }))
    }

    async fn execute_flatten_fetch_node(
        &self,
        node: &FlattenNode,
        representations: Option<Vec<u8>>,
        representation_hashes: Option<Vec<u64>>,
        filtered_representations_hashes: Option<HashMap<u64, usize>>,
    ) -> Result<ExecutionJob, PlanExecutionError> {
        Ok(match node.node.as_ref() {
            PlanNode::Fetch(fetch_node) => ExecutionJob::FlattenFetch(FlattenFetchJob {
                flatten_node_path: node.path.clone(),
                response: self
                    .execute_fetch_node(fetch_node, representations)
                    .await?
                    .into(),
                fetch_node_id: fetch_node.id,
                subgraph_name: fetch_node.service_name.clone(),
                representation_hashes: representation_hashes.unwrap_or_default(),
                representation_hash_to_index: filtered_representations_hashes.unwrap_or_default(),
            }),
            _ => ExecutionJob::None,
        })
    }

    async fn execute_fetch_node(
        &self,
        node: &FetchNode,
        representations: Option<Vec<u8>>,
    ) -> Result<ExecutionJob, PlanExecutionError> {
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
            subgraph_name: node.service_name.clone(),
            response: self
                .executors
                .execute(&node.service_name, subgraph_request, self.client_request)
                .await
                .into(),
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

    use super::{format_error_response, select_fetch_variables, GraphQLError};
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

    #[test]
    fn format_error_response_correctly() {
        let error1 = GraphQLError::from_message_and_extensions(
            format!("Entity resolution error: {}", "Oops!"),
            Default::default(),
        );
        let error2 = GraphQLError::from_message_and_extensions(
            "Hello World".to_string(),
            Default::default(),
        );

        insta::assert_snapshot!(String::from_utf8(format_error_response(&[error1, error2])).unwrap(), @r#"{"data":null,"errors":[{"message":"Entity resolution error: Oops!"},{"message":"Hello World"}]}"#);
    }
}
