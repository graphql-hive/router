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
    execution::rewrites::FetchRewriteExt,
    executors::{common::HttpExecutionRequest, map::SubgraphExecutorMap},
    projection::{
        request::{project_requires, RequestProjectionContext},
        response::project_by_operation,
    },
    response::{merge::deep_merge, subgraph_response::SubgraphResponse, value::Value},
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
) -> Bytes {
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
            Some(_) => panic!("Unsupported plan node type"),
            None => panic!("Empty plan"),
        }
    }

    async fn execute_fetch_wave(&self, ctx: &mut ExecutionContext<'a>, node: &FetchNode) {
        let result = self.execute_fetch_node(node, None).await;
        self.process_job_result(ctx, result);
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
            self.process_job_result(ctx, result);
        }
    }

    async fn execute_plan_node(&self, ctx: &mut ExecutionContext<'a>, node: &PlanNode) {
        match node {
            PlanNode::Fetch(fetch_node) => {
                let job = self.execute_fetch_node(fetch_node, None).await;
                self.process_job_result(ctx, job);
            }
            PlanNode::Parallel(parallel_node) => {
                self.execute_parallel_wave(ctx, parallel_node).await;
            }
            PlanNode::Flatten(flatten_node) => {
                if let Some((representations, representation_hashes, filtered_hashes)) =
                    self.prepare_flatten_data(&ctx.final_response, flatten_node)
                {
                    let job = self
                        .execute_flatten_fetch_node(
                            flatten_node,
                            Some(representations),
                            Some(representation_hashes),
                            Some(filtered_hashes),
                        )
                        .await;
                    self.process_job_result(ctx, job);
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
            _ => {
                panic!("Unexpected node in plan execution");
            }
        }
    }

    fn prepare_job_future<'s>(
        &'s self,
        node: &'s PlanNode,
        final_response: &Value<'a>,
    ) -> BoxFuture<'s, ExecutionJob> {
        match node {
            PlanNode::Fetch(fetch_node) => Box::pin(self.execute_fetch_node(fetch_node, None)),
            PlanNode::Flatten(flatten_node) => {
                let result = self.prepare_flatten_data(final_response, flatten_node);

                if result.is_none() {
                    return Box::pin(async { ExecutionJob::None });
                }

                let (representations, representation_hashes, filtered_hashes) = result.unwrap();

                Box::pin(self.execute_flatten_fetch_node(
                    flatten_node,
                    Some(representations),
                    Some(representation_hashes),
                    Some(filtered_hashes),
                ))
            }
            PlanNode::Condition(node) => {
                match condition_node_by_variables(node, self.variable_values) {
                    Some(node) => Box::pin(self.prepare_job_future(node, final_response)), // This is already clean.
                    None => Box::pin(async { ExecutionJob::None }),
                }
            }
            // Our Query Planner does not produce any other plan node types in ParallelNode
            _ => panic!("unexpected node type in parallel wave"),
        }
    }

    fn process_job_result(&self, ctx: &mut ExecutionContext<'a>, job: ExecutionJob) {
        match job {
            ExecutionJob::Fetch(job) => {
                let idx = ctx.response_storage.add_response(job.response);
                let bytes: &'a [u8] =
                    unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };
                let output_rewrites: Option<&'a Vec<FetchRewrite>> =
                    unsafe { std::mem::transmute(ctx.output_rewrites.get(job.fetch_node_id)) };
                let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
                let response = SubgraphResponse::deserialize(&mut deserializer).unwrap();
                let mut data_ref: Value<'a> = unsafe { std::mem::transmute(response.data) };
                ctx.handle_errors(response.errors);

                if let Some(output_rewrites) = output_rewrites {
                    for output_rewrite in output_rewrites {
                        output_rewrite.rewrite(&self.schema_metadata.possible_types, &mut data_ref);
                    }
                }

                deep_merge(&mut ctx.final_response, data_ref);
            }
            ExecutionJob::FlattenFetch(job) => {
                let idx = ctx.response_storage.add_response(job.response);
                let bytes: &'a [u8] =
                    unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };
                let output_rewrites: Option<&'a Vec<FetchRewrite>> =
                    unsafe { std::mem::transmute(ctx.output_rewrites.get(job.fetch_node_id)) };
                let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
                let response = SubgraphResponse::deserialize(&mut deserializer).unwrap();
                let mut data = response.data;
                let mut entities: Vec<Value<'a>> =
                    unsafe { std::mem::transmute(data.as_entities().unwrap()) };
                ctx.handle_errors(response.errors);

                if let Some(output_rewrites) = output_rewrites {
                    for output_rewrite in output_rewrites {
                        for entity in &mut entities {
                            output_rewrite.rewrite(&self.schema_metadata.possible_types, entity);
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
                        if let Some(entity_index) = job.representation_hash_to_index.get(&hash) {
                            if let Some(entity) = entities.get(*entity_index) {
                                let new_val: Value<'_> =
                                    unsafe { std::mem::transmute(entity.clone()) };
                                deep_merge(target, new_val);
                            }
                        }
                        index += 1;
                    },
                );
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
    ) -> Option<(BytesMut, Vec<u64>, HashMap<u64, usize>)> {
        let fetch_node = match flatten_node.node.as_ref() {
            PlanNode::Fetch(fetch_node) => fetch_node,
            _ => panic!("FlattenNode can only have FetchNode as child"),
        };
        let requires_nodes = fetch_node.requires.as_ref().unwrap();

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
                let hash = entity.to_hash();

                if !entity.is_null() {
                    representation_hashes.push(hash);
                }

                if filtered_representations_hashes.contains_key(&hash) {
                    return;
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
                );

                if is_projected {
                    indexes.insert(index);
                    filtered_representations_hashes.insert(hash, index);
                }
                index += 1;
            },
        );
        filtered_representations.put(CLOSE_BRACKET);

        if indexes.is_empty() {
            return None;
        }

        Some((
            filtered_representations,
            representation_hashes,
            filtered_representations_hashes,
        ))
    }

    async fn execute_flatten_fetch_node(
        &self,
        node: &FlattenNode,
        representations: Option<BytesMut>,
        representation_hashes: Option<Vec<u64>>,
        filtered_representations_hashes: Option<HashMap<u64, usize>>,
    ) -> ExecutionJob {
        match node.node.as_ref() {
            PlanNode::Fetch(fetch_node) => ExecutionJob::FlattenFetch(FlattenFetchJob {
                flatten_node_path: node.path.clone(),
                response: self
                    .execute_fetch_node(fetch_node, representations)
                    .await
                    .into(),
                fetch_node_id: fetch_node.id,
                representation_hashes: representation_hashes.unwrap_or_default(),
                representation_hash_to_index: filtered_representations_hashes.unwrap_or_default(),
            }),
            _ => panic!("FlattenNode can only have FetchNode as child"),
        }
    }

    async fn execute_fetch_node(
        &self,
        node: &FetchNode,
        representations: Option<BytesMut>,
    ) -> ExecutionJob {
        ExecutionJob::Fetch(FetchJob {
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
        })
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
