use std::collections::{HashMap, VecDeque};

use bytes::{BufMut, Bytes, BytesMut};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use query_planner::planner::plan_nodes::{
    ConditionNode, FetchNode, FetchRewrite, FlattenNode, FlattenNodePath, FlattenNodePathSegment,
    ParallelNode, PlanNode, QueryPlan, SequenceNode,
};
use serde::Deserialize;
use sonic_rs::ValueRef;

use crate::{
    context::QueryPlanExecutionContext,
    execution::{error::PlanExecutionError, rewrites::FetchRewriteExt},
    executors::{common::SubgraphExecutionRequest, map::SubgraphExecutorMap},
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
        error_normalization::{add_subgraph_info_to_error, normalize_errors_for_representations},
        graphql_error::GraphQLError,
        merge::deep_merge,
        subgraph_response::SubgraphResponse,
        value::Value,
    },
    utils::{
        consts::{CLOSE_BRACKET, OPEN_BRACKET, TYPENAME_FIELD_NAME},
        traverse::traverse_and_callback,
    },
};

pub struct ExecuteQueryPlanParams<'exec> {
    pub query_plan: &'exec QueryPlan,
    pub projection_plan: &'exec Vec<FieldProjectionPlan>,
    pub variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    pub extensions: Option<HashMap<String, sonic_rs::Value>>,
    pub introspection_context: &'exec IntrospectionContext<'exec, 'static>,
    pub operation_type_name: &'exec str,
    pub executors: &'exec SubgraphExecutorMap,
}

pub async fn execute_query_plan<'exec>(
    ctx: ExecuteQueryPlanParams<'exec>,
) -> Result<Bytes, PlanExecutionError> {
    let init_value = if let Some(introspection_query) = ctx.introspection_context.query {
        resolve_introspection(introspection_query, ctx.introspection_context)
    } else {
        Value::Null
    };

    let mut exec_ctx = QueryPlanExecutionContext::new(ctx.query_plan, init_value);

    if ctx.query_plan.node.is_some() {
        let executor = QueryPlanExecutor::new(
            ctx.variable_values,
            ctx.executors,
            ctx.introspection_context.metadata,
        );
        executor
            .execute(&mut exec_ctx, ctx.query_plan.node.as_ref())
            .await;
    }

    let final_response = &exec_ctx.final_response;
    project_by_operation(
        final_response,
        exec_ctx.errors,
        &ctx.extensions,
        ctx.operation_type_name,
        ctx.projection_plan,
        ctx.variable_values,
    )
    .map_err(|e| e.into())
}

pub struct QueryPlanExecutor<'exec> {
    variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
    schema_metadata: &'exec SchemaMetadata,
    subgraph_executors: &'exec SubgraphExecutorMap,
}

struct FetchJob {
    subgraph_name: String,
    fetch_node_id: i64,
    response: Bytes,
}

struct FlattenFetchJob {
    subgraph_name: String,
    flatten_node_path: FlattenNodePath,
    response: Bytes,
    fetch_node_id: i64,
    representation_hashes: Vec<u64>,
    filtered_representations_hashes: HashMap<u64, Vec<VecDeque<usize>>>,
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
    filtered_representations_hashes: HashMap<u64, Vec<VecDeque<usize>>>,
}

impl<'exec> QueryPlanExecutor<'exec> {
    pub fn new(
        variable_values: &'exec Option<HashMap<String, sonic_rs::Value>>,
        executors: &'exec SubgraphExecutorMap,
        schema_metadata: &'exec SchemaMetadata,
    ) -> Self {
        QueryPlanExecutor {
            variable_values,
            subgraph_executors: executors,
            schema_metadata,
        }
    }

    pub async fn execute(
        &self,
        ctx: &mut QueryPlanExecutionContext<'exec>,
        plan: Option<&PlanNode>,
    ) {
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

    async fn execute_fetch_wave(
        &self,
        ctx: &mut QueryPlanExecutionContext<'exec>,
        node: &FetchNode,
    ) {
        match self.execute_fetch_node(node, None).await {
            Ok(result) => self.process_job_result(ctx, result),
            Err(err) => ctx.errors.push(GraphQLError {
                message: err.to_string(),
                locations: None,
                path: None,
                extensions: None,
            }),
        }
    }

    async fn execute_sequence_wave(
        &self,
        ctx: &mut QueryPlanExecutionContext<'exec>,
        node: &SequenceNode,
    ) {
        for child in &node.nodes {
            Box::pin(self.execute_plan_node(ctx, child)).await;
        }
    }

    async fn execute_parallel_wave(
        &self,
        ctx: &mut QueryPlanExecutionContext<'exec>,
        node: &ParallelNode,
    ) {
        let mut scope = FuturesUnordered::new();

        for child in &node.nodes {
            let job_future = self.prepare_job_future(child, &ctx.final_response);
            scope.push(job_future);
        }

        while let Some(result) = scope.next().await {
            match result {
                Ok(job) => {
                    self.process_job_result(ctx, job);
                }
                Err(err) => ctx.errors.push(GraphQLError {
                    message: err.to_string(),
                    locations: None,
                    path: None,
                    extensions: None,
                }),
            }
        }
    }

    async fn execute_plan_node(&self, ctx: &mut QueryPlanExecutionContext<'exec>, node: &PlanNode) {
        match node {
            PlanNode::Fetch(fetch_node) => match self.execute_fetch_node(fetch_node, None).await {
                Ok(job) => {
                    self.process_job_result(ctx, job);
                }
                Err(err) => ctx.errors.push(GraphQLError {
                    message: err.to_string(),
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
                                Some(p.filtered_representations_hashes),
                            )
                            .await
                        {
                            Ok(job) => {
                                self.process_job_result(ctx, job);
                            }
                            Err(err) => {
                                ctx.errors.push(GraphQLError {
                                    message: err.to_string(),
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
                        Some(p.filtered_representations_hashes),
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
        ctx: &mut QueryPlanExecutionContext<'exec>,
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

        Some((response, output_rewrites))
    }

    fn process_job_result(&self, ctx: &mut QueryPlanExecutionContext<'exec>, job: ExecutionJob) {
        match job {
            ExecutionJob::Fetch(job) => {
                if let Some((mut response, output_rewrites)) =
                    self.process_subgraph_response(ctx, job.response, job.fetch_node_id)
                {
                    if let Some(output_rewrites) = output_rewrites {
                        for output_rewrite in output_rewrites {
                            output_rewrite
                                .rewrite(&self.schema_metadata.possible_types, &mut response.data);
                        }
                    }

                    deep_merge(&mut ctx.final_response, response.data);

                    if let Some(errors) = response.errors {
                        for mut error in errors {
                            error = add_subgraph_info_to_error(error, &job.subgraph_name);
                            ctx.errors.push(error);
                        }
                    }
                }
            }
            ExecutionJob::FlattenFetch(mut job) => {
                if let Some((ref mut response, output_rewrites)) =
                    self.process_subgraph_response(ctx, job.response, job.fetch_node_id)
                {
                    if let Some(mut entities) = response.data.take_entities() {
                        if let Some(output_rewrites) = output_rewrites {
                            for output_rewrite in output_rewrites {
                                for entity in &mut entities {
                                    output_rewrite
                                        .rewrite(&self.schema_metadata.possible_types, entity);
                                }
                            }
                        }
                        'entity_loop: for (entity, hash) in entities
                            .into_iter()
                            .zip(job.representation_hashes.iter_mut())
                        {
                            if let Some(target_paths) =
                                job.filtered_representations_hashes.get_mut(hash)
                            {
                                for indexes_in_path in target_paths {
                                    let mut target: &mut Value<'exec> = &mut ctx.final_response;
                                    for path_segment in job.flatten_node_path.as_slice().iter() {
                                        match path_segment {
                                            FlattenNodePathSegment::List => {
                                                let index = indexes_in_path.pop_front().unwrap();
                                                if let Value::Array(arr) = target {
                                                    if let Some(item) = arr.get_mut(index) {
                                                        target = item;
                                                    } else {
                                                        continue 'entity_loop; // Skip if index is out of bounds
                                                    }
                                                } else {
                                                    continue 'entity_loop; // Skip if target is not an array
                                                }
                                            }
                                            FlattenNodePathSegment::Field(field_name) => {
                                                if let Value::Object(map) = target {
                                                    if let Ok(idx) = map.binary_search_by_key(
                                                        &field_name.as_str(),
                                                        |(k, _)| k,
                                                    ) {
                                                        if let Some((_, value)) = map.get_mut(idx) {
                                                            target = value;
                                                        } else {
                                                            continue 'entity_loop;
                                                            // Skip if field not found
                                                        }
                                                    } else {
                                                        continue 'entity_loop; // Skip if field not found
                                                    }
                                                } else {
                                                    continue 'entity_loop; // Skip if target is not an object
                                                }
                                            }
                                            FlattenNodePathSegment::Cast(type_condition) => {
                                                let mut type_name: &str = type_condition;
                                                if let Some(map) = target.as_object() {
                                                    if let Ok(idx) = map.binary_search_by_key(
                                                        &TYPENAME_FIELD_NAME,
                                                        |(k, _)| k,
                                                    ) {
                                                        if let Some((_, type_name_value)) =
                                                            map.get(idx)
                                                        {
                                                            if let Some(type_name_str) =
                                                                type_name_value.as_str()
                                                            {
                                                                type_name = type_name_str;
                                                            }
                                                        }
                                                    }
                                                }
                                                if !self
                                                    .schema_metadata
                                                    .possible_types
                                                    .entity_satisfies_type_condition(
                                                        type_name,
                                                        type_condition,
                                                    )
                                                {
                                                    continue 'entity_loop; // Skip if type condition is not satisfied
                                                }
                                            }
                                        }
                                    }
                                    if !indexes_in_path.is_empty() {
                                        // If there are still indexes left, we need to traverse them
                                        while let Some(index) = indexes_in_path.pop_front() {
                                            if let Value::Array(arr) = target {
                                                if let Some(item) = arr.get_mut(index) {
                                                    target = item;
                                                } else {
                                                    continue 'entity_loop; // Skip if index is out of bounds
                                                }
                                            } else {
                                                continue 'entity_loop; // Skip if target is not an array
                                            }
                                        }
                                    }
                                    let new_val: Value<'_> =
                                        unsafe { std::mem::transmute(entity.clone()) };
                                    deep_merge(target, new_val);
                                }
                            }
                        }
                    }

                    if let Some(errors) = &response.errors {
                        let normalized_errors = normalize_errors_for_representations(
                            &job.subgraph_name,
                            job.flatten_node_path.as_slice(),
                            &job.representation_hashes,
                            &job.filtered_representations_hashes,
                            errors,
                        );
                        ctx.errors.extend(normalized_errors);
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

        let normalized_path = flatten_node.path.as_slice();
        let mut filtered_representations = BytesMut::new();
        filtered_representations.put(OPEN_BRACKET);
        let proj_ctx = RequestProjectionContext::new(&self.schema_metadata.possible_types);
        let mut representation_hashes: Vec<u64> = Vec::new();
        let mut filtered_representations_hashes: HashMap<u64, Vec<VecDeque<usize>>> =
            HashMap::new();
        let arena = bumpalo::Bump::new();
        let mut number_of_indexes = 0;
        for segment in normalized_path.iter() {
            if *segment == FlattenNodePathSegment::List {
                number_of_indexes += 1;
            }
        }
        traverse_and_callback(
            final_response,
            normalized_path,
            self.schema_metadata,
            VecDeque::with_capacity(number_of_indexes),
            &mut |entity: &Value,
                  indexes_in_path: VecDeque<usize>|
             -> Result<(), PlanExecutionError> {
                if entity.is_null() {
                    return Ok(());
                }

                let hash = entity.to_hash(&requires_nodes.items, proj_ctx.possible_types);

                let indexes_in_paths = filtered_representations_hashes.get_mut(&hash);

                match indexes_in_paths {
                    Some(indexes_in_paths) => {
                        indexes_in_paths.push(indexes_in_path);
                    }
                    None => {
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
                            &proj_ctx,
                            &requires_nodes.items,
                            entity,
                            &mut filtered_representations,
                            filtered_representations_hashes.is_empty(),
                            None,
                        )?;

                        if is_projected {
                            representation_hashes.push(hash);
                            filtered_representations_hashes.insert(hash, vec![indexes_in_path]);
                        }
                    }
                }

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
            filtered_representations_hashes,
        }))
    }

    async fn execute_flatten_fetch_node(
        &self,
        node: &FlattenNode,
        representations: Option<BytesMut>,
        representation_hashes: Option<Vec<u64>>,
        filtered_representations_hashes: Option<HashMap<u64, Vec<VecDeque<usize>>>>,
    ) -> Result<ExecutionJob, PlanExecutionError> {
        Ok(match node.node.as_ref() {
            PlanNode::Fetch(fetch_node) => ExecutionJob::FlattenFetch(FlattenFetchJob {
                flatten_node_path: node.path.clone(),
                subgraph_name: fetch_node.service_name.to_string(),
                response: self
                    .execute_fetch_node(fetch_node, representations)
                    .await?
                    .into(),
                fetch_node_id: fetch_node.id,
                representation_hashes: representation_hashes.unwrap_or_default(),
                filtered_representations_hashes: filtered_representations_hashes
                    .unwrap_or_default(),
            }),
            _ => ExecutionJob::None,
        })
    }

    async fn execute_fetch_node(
        &self,
        node: &FetchNode,
        representations: Option<BytesMut>,
    ) -> Result<ExecutionJob, PlanExecutionError> {
        Ok(ExecutionJob::Fetch(FetchJob {
            subgraph_name: node.service_name.to_string(),
            fetch_node_id: node.id,
            response: self
                .subgraph_executors
                .execute(
                    &node.service_name,
                    SubgraphExecutionRequest {
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
    let vars = variable_values.as_ref()?;
    let value = vars.get(&condition_node.condition)?;
    let condition_met = matches!(value.as_ref(), ValueRef::Bool(true));

    if condition_met {
        condition_node.if_clause.as_deref()
    } else {
        condition_node.else_clause.as_deref()
    }
}
