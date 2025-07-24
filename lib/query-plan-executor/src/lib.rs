use async_trait::async_trait;
use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use query_planner::{
    ast::selection_item::SelectionItem,
    planner::plan_nodes::{
        ConditionNode, FetchNode, FetchNodePathSegment, FetchRewrite, FlattenNode,
        FlattenNodePathSegment, KeyRenamer, ParallelNode, PlanNode, QueryPlan, SequenceNode,
        ValueSetter,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::VecDeque;
use std::{collections::HashMap, vec};
use tracing::{instrument, trace, warn}; // For reading file in main

use crate::{
    executors::{
        common::{SubgraphExecutionResult, SubgraphExecutionResultData},
        map::SubgraphExecutorMap,
    },
    json_writer::write_and_escape_string,
    projection::FieldProjectionPlan,
    schema_metadata::{PossibleTypes, SchemaMetadata},
};
pub mod deep_merge;
mod error_normalization;
pub mod executors;
pub mod introspection;
mod json_writer;
pub mod projection;
pub mod schema_metadata;
pub mod validation;
mod value_from_ast;
pub mod variables;

const TYPENAME_FIELD: &str = "__typename";

#[async_trait]
trait ExecutablePlanNode {
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    );
}

#[async_trait]
pub trait ExecutableQueryPlan {
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    );
}

#[async_trait]
impl ExecutablePlanNode for PlanNode {
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        match self {
            PlanNode::Fetch(node) => node.execute(execution_context, data).await,
            PlanNode::Sequence(node) => node.execute(execution_context, data).await,
            PlanNode::Parallel(node) => node.execute(execution_context, data).await,
            PlanNode::Flatten(node) => node.execute(execution_context, data).await,
            PlanNode::Condition(node) => node.execute(execution_context, data).await,
            PlanNode::Subscription(node) => {
                // Subscriptions typically use a different protocol.
                // Execute the primary node for now.
                warn!(
            "Executing SubscriptionNode's primary as a normal node. Real subscription handling requires a different mechanism."
        );
                node.primary.execute(execution_context, data).await
            }
            PlanNode::Defer(_) => {
                // Defer/Deferred execution is complex.
                warn!("DeferNode execution is not fully implemented.");
            }
        }
    }
}

fn process_errors_and_extensions(
    execution_context: &mut QueryPlanExecutionContext<'_>,
    errors: Option<Vec<GraphQLError>>,
    extensions: Option<HashMap<String, Value>>,
) {
    if let Some(errors) = errors {
        trace!("Processing errors: {:?}", errors);
        execution_context.errors.extend(errors);
    }
    // 7. Handle extensions
    if let Some(extensions) = extensions {
        trace!("Processing extensions: {:?}", extensions);
        execution_context.extensions.extend(extensions);
    }
}

#[async_trait]
trait ExecutableFetchNode {
    async fn execute_and_get_result(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        filtered_representations: Option<Vec<u8>>,
    ) -> SubgraphExecutionResult;
    fn prepare_variables_for_fetch_node<'a>(
        &'a self,
        variable_values: &'a Option<HashMap<String, Value>>,
    ) -> Option<HashMap<&'a str, &'a Value>>;
}

#[async_trait]
impl ExecutablePlanNode for FetchNode {
    #[instrument(level = "debug", skip_all, name = "FetchNode::execute")]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        let fetch_result = self.execute_and_get_result(execution_context, None).await;

        // Process root FetchNode results
        if let Some(fetch_result_data) = fetch_result.data {
            fetch_result_data.merge_into(data);
        }

        process_errors_and_extensions(
            execution_context,
            fetch_result.errors,
            fetch_result.extensions,
        );
    }
}

#[async_trait]
impl ExecutableFetchNode for FetchNode {
    #[instrument(level = "debug", skip_all, name = "FetchNode::execute_and_get_result")]
    async fn execute_and_get_result(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        filtered_representations: Option<Vec<u8>>,
    ) -> SubgraphExecutionResult {
        // 2. Prepare variables for fetch
        let execution_request = SubgraphExecutionRequest {
            query: &self.operation.document_str,
            operation_name: self.operation_name.as_deref(),
            variables: self.prepare_variables_for_fetch_node(execution_context.variable_values),
            extensions: None,
            representations: filtered_representations,
        };

        // 3. Execute the fetch operation
        let mut fetch_result = execution_context
            .subgraph_executor_map
            .execute(&self.service_name, execution_request)
            .await;

        if let Some(data) = &mut fetch_result.data {
            // 5. Apply output rewrites
            if let Some(output_rewrites) = &self.output_rewrites {
                for rewrite in output_rewrites {
                    rewrite.apply(
                        &execution_context.schema_metadata.possible_types,
                        FetchRewriteInput::SubgraphExecutionResultData(data),
                    );
                }
            }
        }

        fetch_result
    }

    #[instrument(
        level = "debug",
        skip(self, variable_values),
        name = "prepare_variables_for_fetch_node"
    )]
    fn prepare_variables_for_fetch_node<'a>(
        &'a self,
        variable_values: &'a Option<HashMap<String, Value>>,
    ) -> Option<HashMap<&'a str, &'a Value>> {
        match (&self.variable_usages, variable_values) {
            (Some(ref variable_usages), Some(variable_values)) => {
                if variable_usages.is_empty() || variable_values.is_empty() {
                    None // No variables to prepare
                } else {
                    Some(
                        variable_usages
                            .iter()
                            .filter_map(|variable_name| {
                                variable_values
                                    .get(variable_name)
                                    .map(|v| (variable_name.as_str(), v))
                            })
                            .collect(),
                    )
                }
            }
            _ => None,
        }
    }
}

trait ApplyFetchRewrite {
    fn apply(&self, possible_types: &PossibleTypes, input: FetchRewriteInput);
}

impl ApplyFetchRewrite for FetchRewrite {
    fn apply(&self, possible_types: &PossibleTypes, input: FetchRewriteInput) {
        let path = self.path();
        match input {
            FetchRewriteInput::Value(value) => {
                self.apply_path(possible_types, value, path);
            }
            FetchRewriteInput::SubgraphExecutionResultData(data) => {
                let current_segment = &path[0];
                let remaining_path = &path[1..];
                match &current_segment {
                    FetchNodePathSegment::Key(field) => {
                        if field == "_entities" {
                            if let Some(entities) = &mut data._entities {
                                for entity in entities {
                                    self.apply_path(possible_types, entity, remaining_path);
                                }
                            }
                        } else {
                            let field_val = data.root_fields.get_mut(field);
                            if let Some(field_val) = field_val {
                                self.apply_path(possible_types, field_val, remaining_path);
                            }
                        }
                    }
                    FetchNodePathSegment::TypenameEquals(_) => {
                        unreachable!(
                            "FetchRewrite should not start with TypenameEquals, it should be handled separately"
                        );
                    }
                }
            }
        }
    }
}

enum FetchRewriteInput<'a> {
    Value(&'a mut Value),
    SubgraphExecutionResultData(&'a mut SubgraphExecutionResultData),
}

trait ApplyFetchRewriteImpl {
    fn path(&self) -> &[FetchNodePathSegment];
    fn apply_path(
        &self,
        possible_types: &PossibleTypes,
        value: &mut Value,
        path: &[FetchNodePathSegment],
    );
}

impl ApplyFetchRewriteImpl for FetchRewrite {
    fn path(&self) -> &[FetchNodePathSegment] {
        match self {
            FetchRewrite::KeyRenamer(renamer) => renamer.path(),
            FetchRewrite::ValueSetter(setter) => setter.path(),
        }
    }
    fn apply_path(
        &self,
        possible_types: &PossibleTypes,
        value: &mut Value,
        path: &[FetchNodePathSegment],
    ) {
        match self {
            FetchRewrite::KeyRenamer(renamer) => {
                renamer.apply_path(possible_types, value, path);
            }
            FetchRewrite::ValueSetter(setter) => {
                setter.apply_path(possible_types, value, path);
            }
        }
    }
}

impl ApplyFetchRewriteImpl for KeyRenamer {
    fn path(&self) -> &[FetchNodePathSegment] {
        &self.path
    }
    // Applies key rename operation on a Value (mutably)
    fn apply_path(
        &self,
        possible_types: &PossibleTypes,
        value: &mut Value,
        path: &[FetchNodePathSegment],
    ) {
        let current_segment = &path[0];
        let remaining_path = &path[1..];

        match value {
            Value::Array(arr) => {
                for item in arr {
                    self.apply_path(possible_types, item, path);
                }
            }
            Value::Object(obj) => {
                match current_segment {
                    FetchNodePathSegment::TypenameEquals(type_condition) => {
                        let type_name = match obj.get(TYPENAME_FIELD) {
                            Some(Value::String(type_name)) => type_name,
                            _ => type_condition, // Default to type_condition if not found
                        };
                        if possible_types.entity_satisfies_type_condition(type_name, type_condition)
                        {
                            self.apply_path(possible_types, value, remaining_path)
                        }
                    }
                    FetchNodePathSegment::Key(field_name) => {
                        if remaining_path.is_empty() {
                            if *field_name != self.rename_key_to {
                                if let Some(val) = obj.remove(field_name) {
                                    obj.insert(self.rename_key_to.to_string(), val);
                                }
                            }
                        } else if let Some(next_value) = obj.get_mut(field_name) {
                            self.apply_path(possible_types, next_value, remaining_path)
                        }
                    }
                }
            }
            _ => (),
        }
    }
}

impl ApplyFetchRewriteImpl for ValueSetter {
    fn path(&self) -> &[FetchNodePathSegment] {
        &self.path
    }
    // Applies value setting on a Value (returns a new Value)
    fn apply_path(
        &self,
        possible_types: &PossibleTypes,
        data: &mut Value,
        path: &[FetchNodePathSegment],
    ) {
        if path.is_empty() {
            *data = self.set_value_to.to_owned();
            return;
        }

        match data {
            Value::Array(arr) => {
                for data in arr {
                    // Apply the path to each item in the array
                    self.apply_path(possible_types, data, path);
                }
            }
            Value::Object(map) => {
                let current_segment = &path[0];
                let remaining_path = &path[1..];

                match current_segment {
                    FetchNodePathSegment::TypenameEquals(type_condition) => {
                        let type_name = match map.get(TYPENAME_FIELD) {
                            Some(Value::String(type_name)) => type_name,
                            _ => type_condition, // Default to type_condition if not found
                        };
                        if possible_types.entity_satisfies_type_condition(type_name, type_condition)
                        {
                            self.apply_path(possible_types, data, remaining_path)
                        }
                    }
                    FetchNodePathSegment::Key(field_name) => {
                        if let Some(data) = map.get_mut(field_name) {
                            // If the key exists, apply the remaining path to its value
                            self.apply_path(possible_types, data, remaining_path)
                        }
                    }
                }
            }
            _ => {
                warn!(
                    "Trying to apply ValueSetter path {:?} to non-object/array type: {:?}",
                    path, data
                );
            }
        }
    }
}

impl SubgraphExecutionResultData {
    pub fn merge_into(self, data: &mut Value) {
        // Process root FetchNode results
        if data.is_null() {
            *data = Value::Object(self.root_fields); // Initialize with result_data
        } else {
            for (key, value) in self.root_fields {
                // Merge the root fields into the target data
                if let Some(target_value) = data.get_mut(&key) {
                    deep_merge::deep_merge(target_value, value);
                } else {
                    data[key] = value;
                }
            }
        }
    }
}

#[async_trait]
impl ExecutablePlanNode for SequenceNode {
    #[instrument(level = "trace", skip_all, name = "SequenceNode::execute", fields(
        nodes_count = %self.nodes.len()
    ))]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        for node in &self.nodes {
            node.execute(execution_context, data) // No representations passed to child nodes
                .await;
        }
    }
}

enum ParallelJob<'a> {
    Root(SubgraphExecutionResult),
    Flatten(
        SubgraphExecutionResult,
        &'a [FlattenNodePathSegment],
        Vec<VecDeque<usize>>,
    ),
}

#[async_trait]
impl ExecutablePlanNode for ParallelNode {
    #[instrument(level = "trace", skip_all, name = "ParallelNode::execute", fields(
        nodes_count = %self.nodes.len()
    ))]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        let mut all_errors = vec![];
        let mut all_extensions = vec![];

        {
            let mut jobs: FuturesUnordered<BoxFuture<ParallelJob>> = FuturesUnordered::new();

            // Collect Fetch node results and flatten nodes for parallel execution
            let now = std::time::Instant::now();
            for node in &self.nodes {
                let node = if let PlanNode::Condition(condition_node) = node {
                    // If the node is a ConditionNode, we need to check the condition
                    if let Some(inner_node) =
                        condition_node.inner_node_by_variables(execution_context.variable_values)
                    {
                        inner_node
                    } else {
                        continue; // Skip this node if the condition is not met
                    }
                } else {
                    node
                };
                match node {
                    PlanNode::Fetch(fetch_node) => {
                        let job = fetch_node.execute_and_get_result(execution_context, None);
                        jobs.push(Box::pin(job.map(ParallelJob::Root)));
                    }
                    PlanNode::Flatten(flatten_node) => {
                        let fetch_node = match flatten_node.node.as_ref() {
                            PlanNode::Fetch(fetch_node) => fetch_node,
                            _ => {
                                warn!(
                                "FlattenNode can only execute FetchNode as child node, found: {:?}",
                                flatten_node.node
                            );
                                continue; // Skip if the child node is not a FetchNode
                            }
                        };
                        let mut filtered_representations = Vec::with_capacity(1024);
                        filtered_representations.push(b'[');

                        let (normalized_path, number_of_indexes) =
                            flatten_node.normalized_path_and_number_of_indexes();
                            
                        let mut indexes_in_paths: Vec<VecDeque<usize>> = vec![];
                        traverse_and_callback(
                            data,
                            normalized_path,
                            execution_context.schema_metadata,
                            VecDeque::with_capacity(number_of_indexes),
                            &mut |entity: &mut Value, indexes_in_path| {
                                let is_projected = fetch_node.project_and_rewrite(
                                    entity,
                                    execution_context,
                                    &mut filtered_representations,
                                    indexes_in_paths.is_empty(),
                                );
                                if is_projected {
                                    indexes_in_paths.push(indexes_in_path);
                                }
                            },
                        );
                        filtered_representations.push(b']');
                        let job = fetch_node.execute_and_get_result(
                            execution_context,
                            Some(filtered_representations),
                        );
                        jobs.push(Box::pin(job.map(|r| {
                            ParallelJob::Flatten(r, normalized_path, indexes_in_paths)
                        })));
                    }
                    _ => {}
                }
            }
            trace!("Prepared {} jobs in {:?}", jobs.len(), now.elapsed());

            let now = std::time::Instant::now();
            while let Some(result) = jobs.next().await {
                match result {
                    ParallelJob::Root(fetch_result) => {
                        // Process root FetchNode results
                        if let Some(fetch_result_data) = fetch_result.data {
                            fetch_result_data.merge_into(data);
                        }
                        // Process errors and extensions
                        if let Some(errors) = fetch_result.errors {
                            all_errors.extend(errors);
                        }
                        if let Some(extensions) = fetch_result.extensions {
                            all_extensions.push(extensions);
                        }
                    }
                    ParallelJob::Flatten(result, path, mut indexes_in_paths) => {
                        if let Some(result_data) = result.data {
                            if let Some(entities) = result_data._entities {
                                'entity_loop: for (entity, indexes_in_path) in
                                    entities.into_iter().zip(indexes_in_paths.iter_mut())
                                {
                                    let mut target = &mut *data;
                                    for path_segment in path.iter() {
                                        match path_segment {
                                            FlattenNodePathSegment::List => {
                                                let index = indexes_in_path.pop_front().unwrap();
                                                target = &mut target[index];
                                            }
                                            FlattenNodePathSegment::Field(field_name) => {
                                                target = &mut target[field_name];
                                            }
                                            FlattenNodePathSegment::Cast(type_condition) => {
                                                let type_name = match target.get(TYPENAME_FIELD) {
                                                    Some(Value::String(type_name)) => type_name,
                                                    _ => type_condition, // Default to type_condition if not found
                                                };
                                                if !execution_context
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
                                            target = &mut target[index];
                                        }
                                    }
                                    deep_merge::deep_merge(target, entity);
                                }
                            }
                        }
                        // Process errors and extensions
                        if let Some(errors) = result.errors {
                            let normalized_errors =
                                error_normalization::normalize_errors_for_representations(
                                    &mut indexes_in_paths,
                                    path,
                                    errors,
                                );
                            all_errors.extend(normalized_errors);
                        }
                        if let Some(extensions) = result.extensions {
                            all_extensions.push(extensions);
                        }
                    }
                }
            }

            trace!(
                "Processed {} parallel jobs in {:?}",
                jobs.len(),
                now.elapsed()
            );
        }
        // 6. Process errors and extensions
        process_errors_and_extensions(
            execution_context,
            if all_errors.is_empty() {
                None
            } else {
                Some(all_errors)
            },
            if all_extensions.is_empty() {
                None
            } else {
                Some(all_extensions.into_iter().flatten().collect())
            },
        );
    }
}

trait ProjectAndRewrite {
    fn project_and_rewrite(
        &self,
        entity: &mut Value,
        execution_context: &QueryPlanExecutionContext<'_>,
        filtered_representations: &mut impl std::io::Write,
        is_first: bool,
    ) -> bool;
}

impl ProjectAndRewrite for FetchNode {
    fn project_and_rewrite(
        &self,
        entity: &mut Value,
        execution_context: &QueryPlanExecutionContext<'_>,
        filtered_representations: &mut impl std::io::Write,
        is_first: bool,
    ) -> bool {
        let requires_nodes = self.requires.as_ref().unwrap();
        if let Some(input_rewrites) = &self.input_rewrites {
            // We need to own the value and not modify the original entity
            let mut entity_owned = entity.to_owned();
            for input_rewrite in input_rewrites {
                input_rewrite.apply(
                    &execution_context.schema_metadata.possible_types,
                    FetchRewriteInput::Value(&mut entity_owned),
                );
            }
            execution_context
                .project_requires(
                    &requires_nodes.items,
                    &entity_owned,
                    filtered_representations,
                    is_first,
                    None,
                )
                .unwrap_or(false)
        } else {
            execution_context
                .project_requires(
                    &requires_nodes.items,
                    entity,
                    filtered_representations,
                    is_first,
                    None,
                )
                .unwrap_or(false)
        }
    }
}

trait NormalizedPathAndNumOfIndexes {
    fn normalized_path_and_number_of_indexes(&self) -> (&[FlattenNodePathSegment], usize);
}

impl NormalizedPathAndNumOfIndexes for FlattenNode {
    fn normalized_path_and_number_of_indexes(&self) -> (&[FlattenNodePathSegment], usize) {
        let normalized_path = self.path.as_slice();
        let mut number_of_indexes = 0;
        for segment in normalized_path.iter() {
            if *segment == FlattenNodePathSegment::List {
                number_of_indexes += 1;
            }
        }
        (normalized_path, number_of_indexes)
    }
}

#[async_trait]
impl ExecutablePlanNode for FlattenNode {
    #[instrument(level = "trace", skip_all, name = "FlattenNode::execute", fields(
        path = ?self.path
    ))]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        // Execute the child node. `execution_context` can be borrowed mutably
        // because `collected_representations` borrows `data_for_flatten`, not `execution_context.data`.
        let now = std::time::Instant::now();
        let mut representations = vec![];
        let mut filtered_representations = Vec::with_capacity(1024);
        let fetch_node = match self.node.as_ref() {
            PlanNode::Fetch(fetch_node) => fetch_node,
            _ => {
                warn!(
                    "FlattenNode can only execute FetchNode as child node, found: {:?}",
                    self.node
                );
                return; // Skip if the child node is not a FetchNode
            }
        };
        filtered_representations.push(b'[');
        let (normalized_path, number_of_indexes) =
            self.normalized_path_and_number_of_indexes();
        traverse_and_callback(
            data,
            normalized_path,
            execution_context.schema_metadata,
            VecDeque::with_capacity(number_of_indexes),
            &mut |entity: &mut Value, indexes_in_paths| {
                let is_projected = fetch_node.project_and_rewrite(
                    entity,
                    execution_context,
                    &mut filtered_representations,
                    representations.is_empty(),
                );
                if is_projected {
                    representations.push((entity, indexes_in_paths));
                }
            },
        );
        filtered_representations.push(b']');
        trace!(
            "traversed and collected representations: {:?} in {:#?}",
            representations.len(),
            now.elapsed()
        );
        if representations.is_empty() {
            // No representations collected, so we skip the fetch execution
            return;
        }
        let result = fetch_node
            .execute_and_get_result(execution_context, Some(filtered_representations))
            .await;

        if let Some(data) = result.data {
            if let Some(entities) = data._entities {
                for (entity, (target, _paths)) in
                    entities.into_iter().zip(representations.iter_mut())
                {
                    // Merge the entity into the representation
                    deep_merge::deep_merge(target, entity);
                }
            }
        }

        let normalized_errors = result.errors.map(|errors| {
            let mut indexes_of_paths = representations
                .iter_mut()
                .map(|(_, paths)| paths.clone())
                .collect::<Vec<VecDeque<usize>>>();
            error_normalization::normalize_errors_for_representations(
                &mut indexes_of_paths,
                normalized_path,
                errors,
            )
        });
        process_errors_and_extensions(execution_context, normalized_errors, result.extensions);
    }
}

trait GetInnerNodeByVariables {
    fn inner_node_by_variables(
        &self,
        variables: &Option<HashMap<String, Value>>,
    ) -> Option<&PlanNode>;
}

impl GetInnerNodeByVariables for ConditionNode {
    fn inner_node_by_variables(
        &self,
        variables: &Option<HashMap<String, Value>>,
    ) -> Option<&PlanNode> {
        let condition_value = variables
            .as_ref()
            .and_then(|vars| vars.get(&self.condition))
            .is_some_and(|val| match val {
                Value::Bool(b) => *b,
                _ => false,
            });
        if condition_value {
            if let Some(if_clause) = &self.if_clause {
                Some(if_clause)
            } else {
                None
            }
        } else if let Some(else_clause) = &self.else_clause {
            Some(else_clause)
        } else {
            None
        }
    }
}

#[async_trait]
impl ExecutablePlanNode for ConditionNode {
    #[instrument(level = "trace", skip_all, name = "ConditionNode::execute")]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        let inner_node = self.inner_node_by_variables(execution_context.variable_values);
        if let Some(node) = inner_node {
            // Execute the inner node if it exists
            node.execute(execution_context, data).await;
        } else {
            // If no inner node, we do nothing
            trace!("ConditionNode condition not met, skipping execution.");
        }
    }
}

#[async_trait]
impl ExecutableQueryPlan for QueryPlan {
    #[instrument(level = "trace", skip_all, name = "QueryPlan::execute")]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        if let Some(root_node) = &self.node {
            root_node.execute(execution_context, data).await
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ExecutionResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<GraphQLError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, Value>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GraphQLError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<GraphQLErrorLocation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<Value>>, // Path can be string or number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, Value>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GraphQLErrorLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone)]
pub struct SubgraphExecutionRequest<'a> {
    pub query: &'a str,
    pub operation_name: Option<&'a str>,
    pub variables: Option<HashMap<&'a str, &'a Value>>,
    pub extensions: Option<HashMap<String, Value>>,
    pub representations: Option<Vec<u8>>,
}

pub struct QueryPlanExecutionContext<'a> {
    // Using `Value` provides flexibility
    pub variable_values: &'a Option<HashMap<String, Value>>,
    pub schema_metadata: &'a SchemaMetadata,
    pub subgraph_executor_map: &'a SubgraphExecutorMap,
    pub errors: Vec<GraphQLError>,
    pub extensions: HashMap<String, Value>,
}

impl QueryPlanExecutionContext<'_> {
    pub fn project_requires(
        &self,
        requires_selections: &Vec<SelectionItem>,
        entity: &Value,
        writer: &mut impl std::io::Write,
        first: bool,
        response_key: Option<&str>,
    ) -> std::io::Result<bool> {
        match entity {
            Value::Null => {
                return Ok(false);
            }
            Value::Bool(b) => {
                if !first {
                    writer.write_all(b",")?;
                }
                if let Some(response_key) = response_key {
                    writer.write_all(b"\"")?;
                    writer.write_all(response_key.as_bytes())?;
                    writer.write_all(b"\"")?;
                    writer.write_all(b":")?;
                    if *b {
                        writer.write_all(b"true")?;
                    } else {
                        writer.write_all(b"false")?;
                    }
                } else if *b {
                    writer.write_all(b"true")?;
                } else {
                    writer.write_all(b"false")?;
                }
            }
            Value::Number(n) => {
                if !first {
                    writer.write_all(b",")?;
                }
                if let Some(response_key) = response_key {
                    writer.write_all(b"\"")?;
                    writer.write_all(response_key.as_bytes())?;
                    writer.write_all(b"\":")?;
                }

                std::io::Write::write_fmt(writer, format_args!("{}", n))?;
            }
            Value::String(s) => {
                if !first {
                    writer.write_all(b",")?;
                }
                if let Some(response_key) = response_key {
                    writer.write_all(b"\"")?;
                    writer.write_all(response_key.as_bytes())?;
                    writer.write_all(b"\":")?;
                }
                write_and_escape_string(writer, s)?;
            }
            Value::Array(entity_array) => {
                if !first {
                    writer.write_all(b",")?;
                }
                if let Some(response_key) = response_key {
                    writer.write_all(b"\"")?;
                    writer.write_all(response_key.as_bytes())?;
                    writer.write_all(b"\":[")?;
                } else {
                    writer.write_all(b"[")?;
                }
                let mut first = true;
                for entity_item in entity_array {
                    let projected = self.project_requires(
                        requires_selections,
                        entity_item,
                        writer,
                        first,
                        None,
                    )?;
                    if projected {
                        // Only update `first` if we actually write something
                        first = false;
                    }
                }
                writer.write_all(b"]")?;
            }
            Value::Object(entity_obj) => {
                if requires_selections.is_empty() {
                    // It is probably a scalar with an object value, so we write it directly
                    serde_json::to_writer(writer, entity_obj)?;
                    return Ok(true);
                }
                if entity_obj.is_empty() {
                    return Ok(false);
                }

                let parent_first = first;
                let mut first = true;
                self.project_requires_map_mut(
                    requires_selections,
                    entity_obj,
                    writer,
                    &mut first,
                    response_key,
                    parent_first,
                )?;
                if first {
                    // If no fields were projected, "first" is still true,
                    // so we skip writing the closing brace
                    return Ok(false);
                } else {
                    writer.write_all(b"}")?;
                }
            }
        };
        Ok(true)
    }

    fn project_requires_map_mut(
        &self,
        requires_selections: &Vec<SelectionItem>,
        entity_obj: &Map<String, Value>,
        writer: &mut impl std::io::Write,
        first: &mut bool,
        parent_response_key: Option<&str>,
        parent_first: bool,
    ) -> std::io::Result<()> {
        for requires_selection in requires_selections {
            match &requires_selection {
                SelectionItem::Field(requires_selection) => {
                    let field_name = &requires_selection.name;
                    let response_key = requires_selection.selection_identifier();
                    if response_key == TYPENAME_FIELD {
                        // Skip __typename field, it is handled separately
                        continue;
                    }

                    let original = entity_obj
                        .get(field_name)
                        .unwrap_or(entity_obj.get(response_key).unwrap_or(&Value::Null));

                    if original.is_null() {
                        continue;
                    }

                    if *first {
                        if !parent_first {
                            writer.write_all(b",")?;
                        }
                        if let Some(parent_response_key) = parent_response_key {
                            writer.write_all(b"\"")?;
                            writer.write_all(parent_response_key.as_bytes())?;
                            writer.write_all(b"\":")?;
                        }
                        writer.write_all(b"{")?;
                        // Write __typename only if the object has other fields
                        if let Some(Value::String(type_name)) = entity_obj.get(TYPENAME_FIELD) {
                            writer.write_all(b"\"__typename\":")?;
                            write_and_escape_string(writer, type_name)?;
                            writer.write_all(b",")?;
                        }
                    }

                    // To avoid writing empty fields, we write to a temporary buffer first
                    self.project_requires(
                        &requires_selection.selections.items,
                        original,
                        writer,
                        *first,
                        Some(response_key),
                    )?;
                    *first = false;
                }
                SelectionItem::InlineFragment(requires_selection) => {
                    let type_condition = &requires_selection.type_condition;

                    let type_name = match entity_obj.get(TYPENAME_FIELD) {
                        Some(Value::String(type_name)) => type_name,
                        _ => type_condition,
                    };
                    // For projection, both sides of the condition are valid
                    if self
                        .schema_metadata
                        .possible_types
                        .entity_satisfies_type_condition(type_name, type_condition)
                        || self
                            .schema_metadata
                            .possible_types
                            .entity_satisfies_type_condition(type_condition, type_name)
                    {
                        self.project_requires_map_mut(
                            &requires_selection.selections.items,
                            entity_obj,
                            writer,
                            first,
                            parent_response_key,
                            parent_first,
                        )?;
                    }
                }
                SelectionItem::FragmentSpread(_name_ref) => {
                    // We only minify the queries to subgraphs, so we never have fragment spreads here
                    unreachable!("Fragment spreads should not exist in FetchNode::requires.");
                }
            }
        }
        Ok(())
    }
}

pub fn traverse_and_callback<'a, Callback>(
    current_data: &'a mut Value,
    remaining_path: &[FlattenNodePathSegment],
    schema_metadata: &SchemaMetadata,
    current_indexes: VecDeque<usize>,
    callback: &mut Callback,
) where
    Callback: FnMut(&'a mut Value, VecDeque<usize>),
{
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            // If the path is empty, we call the callback on each item in the array
            // We iterate because we want the entity objects directly
            for (index, item) in arr.iter_mut().enumerate() {
                let mut new_indexes = current_indexes.clone();
                new_indexes.push_back(index);
                callback(item, new_indexes);
            }
        } else {
            // If the path is empty and current_data is not an array, just call the callback
            callback(current_data, current_indexes);
        }
        return;
    }

    match &remaining_path[0] {
        FlattenNodePathSegment::List => {
            // If the key is List, we expect current_data to be an array
            if let Value::Array(arr) = current_data {
                let rest_of_path = &remaining_path[1..];
                for (index, item) in arr.iter_mut().enumerate() {
                    let mut new_indexes = current_indexes.clone();
                    new_indexes.push_back(index);
                    traverse_and_callback(
                        item,
                        rest_of_path,
                        schema_metadata,
                        new_indexes,
                        callback,
                    );
                }
            }
        }
        FlattenNodePathSegment::Field(field_name) => {
            // If the key is Field, we expect current_data to be an object
            if let Value::Object(map) = current_data {
                if let Some(next_data) = map.get_mut(field_name) {
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback(
                        next_data,
                        rest_of_path,
                        schema_metadata,
                        current_indexes,
                        callback,
                    );
                }
            }
        }
        FlattenNodePathSegment::Cast(type_condition) => {
            // If the key is Cast, we expect current_data to be an object or an array
            if let Value::Object(obj) = current_data {
                let type_name = match obj.get(TYPENAME_FIELD) {
                    Some(Value::String(type_name)) => type_name,
                    _ => type_condition, // Default to type_condition if not found
                };
                if schema_metadata
                    .possible_types
                    .entity_satisfies_type_condition(type_name, type_condition)
                {
                    let rest_of_path = &remaining_path[1..];
                    traverse_and_callback(
                        current_data,
                        rest_of_path,
                        schema_metadata,
                        current_indexes,
                        callback,
                    );
                }
            } else if let Value::Array(arr) = current_data {
                // If the current data is an array, we need to check each item
                for (index, item) in arr.iter_mut().enumerate() {
                    let mut new_indexes = current_indexes.clone();
                    new_indexes.push_back(index);
                    traverse_and_callback(
                        item,
                        remaining_path,
                        schema_metadata,
                        new_indexes,
                        callback,
                    );
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExposeQueryPlanMode {
    Yes,
    No,
    DryRun,
}

#[instrument(
    level = "trace",
    skip_all,
    fields(
        query_plan = ?query_plan,
        variable_values = ?variable_values,
    )
)]
#[allow(clippy::too_many_arguments)]
pub async fn execute_query_plan(
    query_plan: &QueryPlan,
    subgraph_executor_map: &SubgraphExecutorMap,
    variable_values: &Option<HashMap<String, Value>>,
    schema_metadata: &SchemaMetadata,
    operation_type_name: &str,
    selections: &Vec<FieldProjectionPlan>,
    has_introspection: bool,
    expose_query_plan: ExposeQueryPlanMode,
) -> std::io::Result<Vec<u8>> {
    let mut result_data = if has_introspection {
        schema_metadata.introspection_query_json.clone()
    } else {
        Value::Null
    };
    let mut result_errors = vec![]; // Initial errors are empty
    let mut result_extensions = if expose_query_plan == ExposeQueryPlanMode::Yes
        || expose_query_plan == ExposeQueryPlanMode::DryRun
    {
        HashMap::from_iter([("queryPlan".to_string(), serde_json::to_value(query_plan)?)])
    } else {
        HashMap::new()
    };
    let mut execution_context = QueryPlanExecutionContext {
        variable_values,
        subgraph_executor_map,
        schema_metadata,
        errors: result_errors,
        extensions: result_extensions,
    };
    if expose_query_plan != ExposeQueryPlanMode::DryRun {
        query_plan
            .execute(&mut execution_context, &mut result_data)
            .await;
    }
    result_errors = execution_context.errors; // Get the final errors from the execution context
    result_extensions = execution_context.extensions; // Get the final extensions from the execution context
    let mut writer = Vec::with_capacity(4096);
    projection::project_by_operation(
        &mut writer,
        &result_data,
        &mut result_errors,
        &result_extensions,
        operation_type_name,
        selections,
        variable_values,
    )?;

    Ok(writer)
}

#[cfg(test)]
mod tests;
