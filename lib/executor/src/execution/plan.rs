use std::collections::{BTreeSet, HashMap};

use bytes::{BufMut, Bytes, BytesMut};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use query_plan_executor::{projection::FieldProjectionPlan, schema_metadata::SchemaMetadata};
use query_planner::planner::plan_nodes::{
    ConditionNode, FetchNode, FetchRewrite, FlattenNode, FlattenNodePath, ParallelNode, PlanNode,
    QueryPlan, SequenceNode,
};
use serde::Deserialize;
use sonic_rs::ValueRef;

use crate::{
    context::ExecutionContext,
    execution::{error::PlanExecutionError, rewrites::FetchRewriteExt},
    executors::{common::HttpExecutionRequest, map::SubgraphExecutorMap},
    projection::{
        request::{project_requires, RequestProjectionContext},
        response::project_by_operation,
    },
    response::{
        graphql_error::GraphQLError, merge::deep_merge, subgraph_response::SubgraphResponse,
        value::Value,
    },
    utils::{
        consts::{CLOSE_BRACKET, OPEN_BRACKET},
        traverse::{traverse_and_callback, traverse_and_callback_mut},
    },
};

pub async fn execute_query_plan(
    query_plan: &QueryPlan,
    projection_plan: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    extensions: Option<HashMap<String, sonic_rs::Value>>,
    schema_metadata: &SchemaMetadata,
    operation_type_name: &str,
    executors: &SubgraphExecutorMap,
) -> Result<Bytes, PlanExecutionError> {
    let mut ctx = ExecutionContext::new(query_plan);
    let executor = Executor::new(variable_values, executors, schema_metadata);
    execute_query_plan_internal(query_plan, executor, &mut ctx).await;
    let final_response = &ctx.final_response;
    project_by_operation(
        final_response,
        ctx.errors,
        &extensions,
        operation_type_name,
        projection_plan,
        variable_values,
    )
    .map_err(|e| e.into())
}

pub async fn execute_query_plan_internal<'a>(
    query_plan: &QueryPlan,
    executor: Executor<'a>,
    ctx: &mut ExecutionContext<'a>,
) {
    executor.execute(ctx, query_plan.node.as_ref()).await;
}

pub struct Executor<'a> {
    variable_values: &'a Option<HashMap<String, sonic_rs::Value>>,
    schema_metadata: &'a SchemaMetadata,
    executors: &'a SubgraphExecutorMap,
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

struct FetchJob {
    fetch_node_id: i64,
    response: Bytes,
}

struct FlattenFetchJob {
    flatten_node_path: FlattenNodePath,
    response: Bytes,
    fetch_node_id: i64,
    representation_hashes: Vec<u64>,
    representation_hash_to_index: HashMap<u64, usize>,
}

enum ExecutionJob {
    Fetch(FetchJob),
    FlattenFetch(FlattenFetchJob),
    None,
}

impl From<ExecutionJob> for Bytes {
    fn from(value: ExecutionJob) -> Self {
        match value {
            ExecutionJob::Fetch(j) => j.response,
            ExecutionJob::FlattenFetch(j) => j.response,
            ExecutionJob::None => Bytes::new(),
        }
    }
}

struct PreparedFlattenData {
    representations: BytesMut,
    representation_hashes: Vec<u64>,
    representation_hash_to_index: HashMap<u64, usize>,
}

impl<'a> Executor<'a> {
    pub fn new(
        variable_values: &'a Option<HashMap<String, sonic_rs::Value>>,
        executors: &'a SubgraphExecutorMap,
        schema_metadata: &'a SchemaMetadata,
    ) -> Self {
        Executor {
            variable_values,
            executors,
            schema_metadata,
        }
    }

    pub async fn execute(&self, ctx: &mut ExecutionContext<'a>, plan: Option<&PlanNode>) {
        match plan {
            Some(PlanNode::Fetch(node)) => self.execute_fetch_wave(ctx, node).await,
            Some(PlanNode::Parallel(node)) => self.execute_parallel_wave(ctx, node).await,
            Some(PlanNode::Sequence(node)) => self.execute_sequence_wave(ctx, node).await,
            // Plans produced by our Query Planner can only start with: Fetch, Sequence or Parallel.
            // Any other node type at the root is not supported, do nothing
            Some(_) => (),
            // An empty plan is valid, just do nothing
            None => (),
        }
    }

    async fn execute_fetch_wave(&self, ctx: &mut ExecutionContext<'a>, node: &FetchNode) {
        match self.execute_fetch_node(node, None).await {
            Ok(result) => self.process_job_result(ctx, result),
            Err(err) => ctx.errors.push(GraphQLError {
                message: err,
                locations: None,
                path: None,
                extensions: None,
            }),
        }
    }

    async fn execute_sequence_wave(&self, ctx: &mut ExecutionContext<'a>, node: &SequenceNode) {
        for child in &node.nodes {
            Box::pin(self.execute_plan_node(ctx, child)).await;
        }
    }

    async fn execute_parallel_wave(&self, ctx: &mut ExecutionContext<'a>, node: &ParallelNode) {
        let mut scope = ConcurrencyScope::new();

        for child in &node.nodes {
            let job_future = self.prepare_job_future(child, &ctx.final_response);
            scope.spawn(job_future);
        }

        let results = scope.join_all().await;

        for result in results {
            match result {
                Ok(job) => {
                    self.process_job_result(ctx, job);
                }
                Err(err) => ctx.errors.push(GraphQLError {
                    message: err,
                    locations: None,
                    path: None,
                    extensions: None,
                }),
            }
        }
    }

    async fn execute_plan_node(&self, ctx: &mut ExecutionContext<'a>, node: &PlanNode) {
        match node {
            PlanNode::Fetch(fetch_node) => match self.execute_fetch_node(fetch_node, None).await {
                Ok(job) => {
                    self.process_job_result(ctx, job);
                }
                Err(err) => ctx.errors.push(GraphQLError {
                    message: err,
                    locations: None,
                    path: None,
                    extensions: None,
                }),
            },
            PlanNode::Parallel(parallel_node) => {
                self.execute_parallel_wave(ctx, parallel_node).await;
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
                                self.process_job_result(ctx, job);
                            }
                            Err(err) => {
                                ctx.errors.push(GraphQLError {
                                    message: err,
                                    locations: None,
                                    path: None,
                                    extensions: None,
                                });
                            }
                        }
                    }
                    Ok(None) => { /* do nothing */ }
                    Err(e) => {
                        ctx.errors.push(GraphQLError {
                            message: e.to_string(),
                            locations: None,
                            path: None,
                            extensions: None,
                        });
                    }
                }
            }
            PlanNode::Sequence(sequence_node) => {
                self.execute_sequence_wave(ctx, sequence_node).await;
            }
            PlanNode::Condition(condition_node) => {
                if let Some(node) =
                    condition_node_by_variables(condition_node, self.variable_values)
                {
                    Box::pin(self.execute_plan_node(ctx, node)).await;
                }
            }
            // An unsupported plan node was found, do nothing.
            _ => {}
        }
    }

    fn prepare_job_future<'s>(
        &'s self,
        node: &'s PlanNode,
        final_response: &Value<'a>,
    ) -> BoxFuture<'s, Result<ExecutionJob, String>> {
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
                    Err(e) => Box::pin(async move { Err(e.to_string()) }),
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
        ctx: &mut ExecutionContext<'a>,
        response_bytes: Bytes,
        fetch_node_id: i64,
    ) -> Option<(Value<'a>, Option<&'a Vec<FetchRewrite>>)> {
        let idx = ctx.response_storage.add_response(response_bytes);
        // SAFETY: The `bytes` are transmuted to the lifetime `'a` of the `ExecutionContext`.
        // This is safe because the `response_storage` is part of the `ExecutionContext` (`ctx`)
        // and will live as long as `'a`. The `Bytes` are stored in an `Arc`, so they won't be
        // dropped until all references are gone. The `Value`s deserialized from this byte
        // slice will borrow from it, and they are stored in `ctx.final_response`, which also
        // lives for `'a`.
        let bytes: &'a [u8] = unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };

        // SAFETY: The `output_rewrites` are transmuted to the lifetime `'a`. This is safe
        // because `output_rewrites` is part of `OutputRewritesStorage` which is owned by
        // `ExecutionContext` and lives for `'a`.
        let output_rewrites: Option<&'a Vec<FetchRewrite>> =
            unsafe { std::mem::transmute(ctx.output_rewrites.get(fetch_node_id)) };

        let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
        let response = match SubgraphResponse::deserialize(&mut deserializer) {
            Ok(response) => response,
            Err(e) => {
                ctx.errors
                    .push(crate::response::graphql_error::GraphQLError {
                        message: format!("Failed to deserialize subgraph response: {}", e),
                        locations: None,
                        path: None,
                        extensions: None,
                    });
                return None;
            }
        };

        ctx.handle_errors(response.errors);

        // The `response.data` has a lifetime tied to `deserializer`, which is tied to `bytes`.
        // Since `bytes` has been transmuted to `'a`, `response.data` can also be considered
        // to have lifetime `'a`.
        let data: Value<'a> = unsafe { std::mem::transmute(response.data) };

        Some((data, output_rewrites))
    }

    fn process_job_result(&self, ctx: &mut ExecutionContext<'a>, job: ExecutionJob) {
        match job {
            ExecutionJob::Fetch(job) => {
                if let Some((mut data, output_rewrites)) =
                    self.process_subgraph_response(ctx, job.response, job.fetch_node_id)
                {
                    if let Some(output_rewrites) = output_rewrites {
                        for output_rewrite in output_rewrites {
                            output_rewrite.rewrite(&self.schema_metadata.possible_types, &mut data);
                        }
                    }

                    deep_merge(&mut ctx.final_response, data);
                }
            }
            ExecutionJob::FlattenFetch(job) => {
                if let Some((mut data, output_rewrites)) =
                    self.process_subgraph_response(ctx, job.response, job.fetch_node_id)
                {
                    if let Some(entities) = data.take_entities() {
                        // SAFETY: The `entities` vector is transmuted to have lifetime `'a`. This is
                        // safe because the data it contains is borrowed from the response bytes,
                        // which are guaranteed to live for `'a`.
                        let mut entities: Vec<Value<'a>> = unsafe { std::mem::transmute(entities) };

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
                        traverse_and_callback_mut(
                            &mut ctx.final_response,
                            normalized_path,
                            self.schema_metadata,
                            &mut |target| {
                                let hash = job.representation_hashes[index];
                                if let Some(entity_index) =
                                    job.representation_hash_to_index.get(&hash)
                                {
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
                    }
                }
            }
            ExecutionJob::None => {
                // nothing to do
            }
        }
    }

    fn prepare_flatten_data(
        &self,
        final_response: &Value<'a>,
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
        let mut indexes = BTreeSet::new();
        let normalized_path = flatten_node.path.as_slice();
        let mut filtered_representations = BytesMut::new();
        filtered_representations.put(OPEN_BRACKET);
        let proj_ctx = RequestProjectionContext::new(&self.schema_metadata.possible_types);
        let mut representation_hashes: Vec<u64> = Vec::new();
        let mut filtered_representations_hashes: HashMap<u64, usize> = HashMap::new();

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

                let arena = bumpalo::Bump::new();

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
                    indexes.is_empty(),
                    None,
                )?;

                if is_projected {
                    indexes.insert(index);
                    filtered_representations_hashes.insert(hash, index);
                }

                index += 1;

                Ok(())
            },
        )?;
        filtered_representations.put(CLOSE_BRACKET);

        if indexes.is_empty() {
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
        representations: Option<BytesMut>,
        representation_hashes: Option<Vec<u64>>,
        filtered_representations_hashes: Option<HashMap<u64, usize>>,
    ) -> Result<ExecutionJob, String> {
        Ok(match node.node.as_ref() {
            PlanNode::Fetch(fetch_node) => ExecutionJob::FlattenFetch(FlattenFetchJob {
                flatten_node_path: node.path.clone(),
                response: self
                    .execute_fetch_node(fetch_node, representations)
                    .await?
                    .into(),
                fetch_node_id: fetch_node.id,
                representation_hashes: representation_hashes.unwrap_or_default(),
                representation_hash_to_index: filtered_representations_hashes.unwrap_or_default(),
            }),
            _ => ExecutionJob::None,
        })
    }

    async fn execute_fetch_node(
        &self,
        node: &FetchNode,
        representations: Option<BytesMut>,
    ) -> Result<ExecutionJob, String> {
        Ok(ExecutionJob::Fetch(FetchJob {
            fetch_node_id: node.id,
            response: self
                .executors
                .execute(
                    &node.service_name,
                    HttpExecutionRequest {
                        query: node.operation.document_str.as_str(),
                        operation_name: node.operation_name.as_deref(),
                        variables: None,
                        representations,
                    },
                )
                .await,
        }))
    }
}

fn condition_node_by_variables<'a>(
    condition_node: &'a ConditionNode,
    variable_values: &'a Option<HashMap<String, sonic_rs::Value>>,
) -> Option<&'a PlanNode> {
    let condition_value = variable_values
        .as_ref()
        .and_then(|vars| vars.get(&condition_node.condition))
        .is_some_and(|val| match val.as_ref() {
            ValueRef::Bool(b) => b,
            _ => false,
        });
    if condition_value {
        if let Some(if_clause) = &condition_node.if_clause {
            Some(if_clause)
        } else {
            None
        }
    } else if let Some(else_clause) = &condition_node.else_clause {
        Some(else_clause)
    } else {
        None
    }
}
