use std::{
    collections::{BTreeSet, HashMap},
    hash::{DefaultHasher, Hash, Hasher},
};

use bytes::{BufMut, Bytes, BytesMut};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use query_plan_executor::{projection::FieldProjectionPlan, schema_metadata::SchemaMetadata};
use query_planner::planner::plan_nodes::{
    FetchNode, FlattenNode, FlattenNodePath, ParallelNode, PlanNode, QueryPlan, SequenceNode,
};
use serde::Deserialize;

use crate::{
    context::ExecutionContext,
    executors::{common::HttpExecutionRequest, map::SubgraphExecutorMap},
    projection::{
        request::{project_requires, RequestProjectionContext},
        response::project_by_operation,
    },
    response::{merge::deep_merge, value::Value},
    utils::{
        consts::{CLOSE_BRACKET, OPEN_BRACKET},
        traverse::traverse_and_callback,
    },
};

pub async fn execute_query_plan(
    query_plan: &QueryPlan,
    projection_plan: &Vec<FieldProjectionPlan>,
    variable_values: &Option<HashMap<String, serde_json::Value>>,
    schema_metadata: &SchemaMetadata,
    operation_type_name: &str,
    executors: &SubgraphExecutorMap,
) -> BytesMut {
    let mut ctx = ExecutionContext::new();
    let executor = Executor::new(&variable_values, &executors, schema_metadata);
    execute_query_plan_internal(query_plan, executor, &mut ctx).await;
    let final_response = &ctx.final_response;
    project_by_operation(
        final_response,
        operation_type_name,
        &projection_plan,
        &variable_values,
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
        _variable_values: &'a Option<HashMap<String, serde_json::Value>>,
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
                    let result = self.execute_fetch_node(fetch_node, None).await;
                    let idx = ctx.response_storage.add_response(result.into());
                    let bytes: &'a [u8] =
                        unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };
                    let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
                    let mut value = Value::deserialize(&mut deserializer).unwrap();
                    let data_ref: Value<'a> =
                        unsafe { std::mem::transmute(value.to_data().unwrap()) };
                    deep_merge(&mut ctx.final_response, data_ref);
                }
                PlanNode::Parallel(parallel_node) => {
                    self.execute_parallel_wave(ctx, parallel_node).await;
                }
                PlanNode::Flatten(flatten_node) => {
                    let fetch_node = match flatten_node.node.as_ref() {
                        PlanNode::Fetch(fetch_node) => fetch_node,
                        _ => panic!("Unsupported plan node type"),
                    };
                    let requires_nodes = fetch_node.requires.as_ref().unwrap();
                    let mut index = 0;
                    let mut indexes = BTreeSet::new();
                    let normalized_path = flatten_node.path.as_slice();
                    let mut filtered_representations = BytesMut::new();
                    filtered_representations.put(OPEN_BRACKET);
                    let proj_ctx =
                        RequestProjectionContext::new(&self.schema_metadata.possible_types);
                    let mut representation_hashes: Vec<u64> = Vec::new();
                    let mut filtered_representations_hashes: HashMap<u64, usize> = HashMap::new();

                    traverse_and_callback(
                        &mut ctx.final_response,
                        normalized_path,
                        self.schema_metadata,
                        &mut |entity| {
                            let mut hasher = DefaultHasher::new();
                            entity.hash(&mut hasher);
                            let hash = hasher.finish();

                            if !entity.is_null() {
                                representation_hashes.push(hash);
                            }

                            if filtered_representations_hashes.contains_key(&hash) {
                                // deduplicate
                                return;
                            }
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
                    let result = self
                        .execute_flatten_fetch_node(
                            flatten_node,
                            Some(filtered_representations),
                            None,
                            None,
                        )
                        .await;
                    let idx = ctx.response_storage.add_response(result.into());
                    let bytes: &'a [u8] =
                        unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };
                    let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
                    let mut value = Value::deserialize(&mut deserializer).unwrap();
                    let entities: Vec<Value<'a>> =
                        unsafe { std::mem::transmute(value.to_entities().unwrap()) };

                    let mut index = 0;
                    traverse_and_callback(
                        &mut ctx.final_response,
                        normalized_path,
                        self.schema_metadata,
                        &mut |target| {
                            let hash = representation_hashes[index];
                            let entity_index = filtered_representations_hashes.get(&hash).unwrap();
                            let entity = entities.get(*entity_index).unwrap();
                            let new_val: Value<'_> = unsafe { std::mem::transmute(entity.clone()) };
                            deep_merge(target, new_val);
                            index += 1;
                        },
                    );
                }
                _ => panic!("Unsupported plan node type"),
            }
        }
    }

    async fn execute_parallel_wave(&self, ctx: &mut ExecutionContext<'a>, node: &ParallelNode) {
        let mut jobs: FuturesUnordered<BoxFuture<'_, ExecutionJob>> = FuturesUnordered::new();
        // let mut pointers_per_job = Vec::new();

        for child in &node.nodes {
            match child {
                PlanNode::Fetch(fetch_node) => {
                    jobs.push(Box::pin(self.execute_fetch_node(&fetch_node, None)));
                }
                PlanNode::Flatten(flatten_node) => {
                    let fetch_node = match flatten_node.node.as_ref() {
                        PlanNode::Fetch(fetch_node) => fetch_node,
                        _ => panic!("Unsupported plan node type"),
                    };
                    let requires_nodes = fetch_node.requires.as_ref().unwrap();
                    let mut index = 0;
                    let mut indexes = BTreeSet::new();
                    let normalized_path = flatten_node.path.as_slice();
                    let mut filtered_representations = BytesMut::new();
                    filtered_representations.put(OPEN_BRACKET);
                    let proj_ctx =
                        RequestProjectionContext::new(&self.schema_metadata.possible_types);

                    let mut representation_hashes: Vec<u64> = Vec::new();
                    let mut filtered_representations_hashes: HashMap<u64, usize> = HashMap::new();

                    traverse_and_callback(
                        &mut ctx.final_response,
                        normalized_path,
                        self.schema_metadata,
                        &mut |entity| {
                            let mut hasher = DefaultHasher::new();
                            entity.hash(&mut hasher);
                            let hash = hasher.finish();

                            if !entity.is_null() {
                                representation_hashes.push(hash);
                            }

                            if filtered_representations_hashes.contains_key(&hash) {
                                // deduplicate
                                return;
                            }

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
                    jobs.push(Box::pin(self.execute_flatten_fetch_node(
                        flatten_node,
                        Some(filtered_representations),
                        Some(representation_hashes),
                        Some(filtered_representations_hashes),
                    )));
                }
                _ => panic!("unexpected node type"),
            }
        }

        let mut results = Vec::with_capacity(jobs.len());
        while let Some(result) = jobs.next().await {
            results.push(result);
        }

        for result in results {
            match result {
                ExecutionJob::Fetch(res) => {
                    let idx = ctx.response_storage.add_response(res);
                    let bytes: &'a [u8] =
                        unsafe { std::mem::transmute(ctx.response_storage.get_bytes(idx)) };
                    let mut deserializer = sonic_rs::Deserializer::from_slice(bytes);
                    let mut value = Value::deserialize(&mut deserializer).unwrap();
                    let data_ref: Value<'a> =
                        unsafe { std::mem::transmute(value.to_data().unwrap()) };
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
                    traverse_and_callback(
                        &mut ctx.final_response,
                        normalized_path,
                        self.schema_metadata,
                        &mut |target| {
                            let hash = representation_hashes[index];
                            let entity_index =
                                filtered_representations_to_index_map.get(&hash).unwrap();
                            let entity = entities.get(*entity_index).unwrap();
                            let new_val: Value<'_> = unsafe { std::mem::transmute(entity.clone()) };
                            deep_merge(target, new_val);
                            index += 1;
                        },
                    );
                }
            }
        }
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
            _ => panic!("unexpected node type"),
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
                        operation_name: node.operation_name.as_ref().map(|s| s.as_str()),
                        variables: None,
                        representations,
                    },
                )
                .await,
        )
    }
}
