use std::collections::{BTreeSet, HashMap};

use bytes::{BufMut, Bytes, BytesMut};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use query_plan_executor::{projection::FieldProjectionPlan, schema_metadata::SchemaMetadata};
use query_planner::planner::plan_nodes::{
    FetchNode, FlattenNode, FlattenNodePath, ParallelNode, PlanNode, QueryPlan, SequenceNode,
};
use serde::Deserialize;

use crate::{
    context::ExecutionContext,
    execution::rewrites::FetchRewriteExt,
    executors::{common::HttpExecutionRequest, map::SubgraphExecutorMap},
    projection::{
        request::{project_requires, RequestProjectionContext},
        response::project_by_operation,
    },
    response::{merge::deep_merge, value::Value},
    utils::{
        consts::{CLOSE_BRACKET, OPEN_BRACKET},
        traverse::{traverse_and_callback, traverse_and_callback_mut},
    },
};

pub async fn execute_query_plan(
    query_plan: &QueryPlan,
    projection_plan: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, sonic_rs::Value>>,
    schema_metadata: &SchemaMetadata,
    operation_type_name: &str,
    executors: &SubgraphExecutorMap,
) -> BytesMut {
    let mut ctx = ExecutionContext::new();
    let executor = Executor::new(variable_values, executors, schema_metadata);
    execute_query_plan_internal(query_plan, executor, &mut ctx).await;
    let final_response = &ctx.final_response;
    project_by_operation(
        final_response,
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
    // variable_values: &'a Option<HashMap<String, serde_json::Value>>,
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

enum ExecutionJob {
    Fetch(Bytes),
    FlattenFetch(FlattenNodePath, Bytes, Vec<u64>, HashMap<u64, usize>),
}

impl From<ExecutionJob> for Bytes {
    fn from(value: ExecutionJob) -> Self {
        match value {
            ExecutionJob::Fetch(p) => p,
            ExecutionJob::FlattenFetch(_, p, _, _) => p,
        }
    }
}

impl<'a> Executor<'a> {
    pub fn new(
        _variable_values: &'a Option<HashMap<String, sonic_rs::Value>>,
        executors: &'a SubgraphExecutorMap,
        schema_metadata: &'a SchemaMetadata,
    ) -> Self {
        Executor {
            // variable_values,
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
        let idx = ctx.response_storage.add_response(result.into());
        let bytes: &'a [u8] = unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };
        let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
        let mut value = Value::deserialize(&mut deserializer).unwrap();
        let data_ref: Value<'a> = unsafe { std::mem::transmute(value.to_data().unwrap()) };
        ctx.final_response = data_ref;
    }

    async fn execute_sequence_wave(&self, ctx: &mut ExecutionContext<'a>, node: &SequenceNode) {
        for child in &node.nodes {
            match child {
                PlanNode::Fetch(fetch_node) => {
                    let job = self.execute_fetch_node(fetch_node, None).await;
                    self.process_job_result(ctx, job);
                }
                PlanNode::Parallel(parallel_node) => {
                    self.execute_parallel_wave(ctx, parallel_node).await;
                }
                PlanNode::Flatten(flatten_node) => {
                    let (representations, representation_hashes, filtered_hashes) =
                        self.prepare_flatten_data(&ctx.final_response, flatten_node);

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
                // Our Query Planner does not produce any other plan node types in SequenceNode
                _ => panic!("Unsupported plan node type in SequenceNode"),
            }
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

    fn prepare_job_future<'s>(
        &'s self,
        node: &'s PlanNode,
        final_response: &Value<'a>,
    ) -> BoxFuture<'s, ExecutionJob> {
        match node {
            PlanNode::Fetch(fetch_node) => Box::pin(self.execute_fetch_node(fetch_node, None)),
            PlanNode::Flatten(flatten_node) => {
                let (representations, representation_hashes, filtered_hashes) =
                    self.prepare_flatten_data(final_response, flatten_node);

                Box::pin(self.execute_flatten_fetch_node(
                    flatten_node,
                    Some(representations),
                    Some(representation_hashes),
                    Some(filtered_hashes),
                ))
            }
            // Our Query Planner does not produce any other plan node types in ParallelNode
            _ => panic!("unexpected node type in parallel wave"),
        }
    }

    fn process_job_result(&self, ctx: &mut ExecutionContext<'a>, job: ExecutionJob) {
        match job {
            ExecutionJob::Fetch(res) => {
                let idx = ctx.response_storage.add_response(res);
                let bytes: &'a [u8] =
                    unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };
                let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
                let mut value = Value::deserialize(&mut deserializer).unwrap();
                let data_ref: Value<'a> = unsafe { std::mem::transmute(value.to_data().unwrap()) };
                deep_merge(&mut ctx.final_response, data_ref);
            }
            ExecutionJob::FlattenFetch(
                path,
                res,
                representation_hashes,
                filtered_representations_to_index_map,
            ) => {
                let idx = ctx.response_storage.add_response(res);
                let bytes: &'a [u8] =
                    unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };
                let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
                let mut value = Value::deserialize(&mut deserializer).unwrap();
                let entities: Vec<Value<'a>> =
                    unsafe { std::mem::transmute(value.to_entities().unwrap()) };

                let mut index = 0;
                let normalized_path = path.as_slice();
                traverse_and_callback_mut(
                    &mut ctx.final_response,
                    normalized_path,
                    self.schema_metadata,
                    &mut |target| {
                        let hash = representation_hashes[index];
                        if let Some(entity_index) = filtered_representations_to_index_map.get(&hash)
                        {
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
        }
    }

    fn prepare_flatten_data(
        &self,
        final_response: &Value<'a>,
        flatten_node: &FlattenNode,
    ) -> (BytesMut, Vec<u64>, HashMap<u64, usize>) {
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
                        input_rewrite.rewrite(
                            &arena,
                            &self.schema_metadata.possible_types,
                            new_entity,
                        );
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
        (
            filtered_representations,
            representation_hashes,
            filtered_representations_hashes,
        )
    }

    async fn execute_flatten_fetch_node(
        &self,
        node: &FlattenNode,
        representations: Option<BytesMut>,
        representation_hashes: Option<Vec<u64>>,
        filtered_representations_hashes: Option<HashMap<u64, usize>>,
    ) -> ExecutionJob {
        match node.node.as_ref() {
            PlanNode::Fetch(fetch_node) => ExecutionJob::FlattenFetch(
                node.path.clone(),
                self.execute_fetch_node(fetch_node, representations)
                    .await
                    .into(),
                representation_hashes.unwrap_or_default(),
                filtered_representations_hashes.unwrap_or_default(),
            ),
            _ => panic!("FlattenNode can only have FetchNode as child"),
        }
    }

    async fn execute_fetch_node(
        &self,
        node: &FetchNode,
        representations: Option<BytesMut>,
    ) -> ExecutionJob {
        ExecutionJob::Fetch(
            self.executors
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
        )
    }
}
