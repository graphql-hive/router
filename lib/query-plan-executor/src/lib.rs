use async_trait::async_trait;
use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use query_planner::{
    ast::{operation::OperationDefinition, selection_item::SelectionItem},
    planner::plan_nodes::{
        ConditionNode, FetchNode, FetchNodePathSegment, FetchRewrite, FlattenNode, KeyRenamer,
        ParallelNode, PlanNode, QueryPlan, SequenceNode, ValueSetter,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::BTreeSet, fmt::Write};
use std::{collections::HashMap, vec};
use tracing::{instrument, trace, warn}; // For reading file in main

use crate::{
    executors::map::SubgraphExecutorMap,
    json_writer::write_and_escape_string,
    schema_metadata::{PossibleTypes, SchemaMetadata},
};
pub mod deep_merge;
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

#[instrument(
    level = "trace",
    skip(execution_context),
    name = "process_errors_and_extensions"
)]
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
struct ExecuteForRepresentationsResult {
    entities: Option<Vec<Value>>,
    indexes: BTreeSet<usize>,
    errors: Option<Vec<GraphQLError>>,
    extensions: Option<HashMap<String, Value>>,
}

#[async_trait]
trait ExecutableFetchNode {
    async fn execute_for_root(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
    ) -> ExecutionResult;
    async fn execute_for_projected_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        filtered_representations: String,
        indexes: BTreeSet<usize>,
    ) -> ExecuteForRepresentationsResult;
    fn apply_output_rewrites(&self, possible_types: &PossibleTypes, data: &mut Value);
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
        let fetch_result = self.execute_for_root(execution_context).await;

        process_root_result(fetch_result, execution_context, data);
    }
}

#[async_trait]
impl ExecutableFetchNode for FetchNode {
    #[instrument(
        level = "trace",
        skip_all,
        name="FetchNode::execute_for_root",
        fields(
            service_name = self.service_name,
            operation_name = ?self.operation_name,
            operation_str = %self.operation.document_str,
        )
    )]
    async fn execute_for_root(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
    ) -> ExecutionResult {
        let variables = self.prepare_variables_for_fetch_node(execution_context.variable_values);

        let execution_request = SubgraphExecutionRequest {
            query: &self.operation.document_str,
            operation_name: self.operation_name.as_deref(),
            variables,
            extensions: None,
            representations: None,
        };
        let mut fetch_result = execution_context
            .subgraph_executor_map
            .execute(&self.service_name, execution_request)
            .await;

        // 5. Process the response

        if let Some(new_data) = &mut fetch_result.data {
            self.apply_output_rewrites(&execution_context.schema_metadata.possible_types, new_data);
        }

        fetch_result
    }

    #[instrument(
        level = "debug",
        skip_all,
        name = "execute_for_projected_representations",
        fields(
            representations_count = %filtered_representations.len(),
        ),
    )]
    async fn execute_for_projected_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        filtered_representations: String,
        indexes: BTreeSet<usize>,
    ) -> ExecuteForRepresentationsResult {
        // 2. Prepare variables for fetch
        let execution_request = SubgraphExecutionRequest {
            query: &self.operation.document_str,
            operation_name: self.operation_name.as_deref(),
            variables: self.prepare_variables_for_fetch_node(execution_context.variable_values),
            extensions: None,
            representations: Some(filtered_representations),
        };

        // 3. Execute the fetch operation
        let fetch_result = execution_context
            .subgraph_executor_map
            .execute(&self.service_name, execution_request)
            .await;

        // Process data
        let entities = if let Some(mut data) = fetch_result.data {
            self.apply_output_rewrites(
                &execution_context.schema_metadata.possible_types,
                &mut data,
            );
            match data {
                Value::Object(mut obj) => match obj.remove("_entities") {
                    Some(Value::Array(arr)) => Some(arr),
                    _ => None, // If _entities is not found or not an array
                },
                _ => None, // If data is not an object
            }
        } else {
            None
        };
        ExecuteForRepresentationsResult {
            entities,
            indexes,
            errors: fetch_result.errors,
            extensions: fetch_result.extensions,
        }
    }

    fn apply_output_rewrites(&self, possible_types: &PossibleTypes, data: &mut Value) {
        if let Some(output_rewrites) = &self.output_rewrites {
            for rewrite in output_rewrites {
                rewrite.apply(possible_types, data);
            }
        }
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
    fn apply(&self, possible_types: &PossibleTypes, value: &mut Value);
    fn apply_path(&self, possible_types: &PossibleTypes, value: &mut Value, path: &[FetchNodePathSegment]);
}

impl ApplyFetchRewrite for FetchRewrite {
    fn apply(&self, possible_types: &PossibleTypes, value: &mut Value) {
        match self {
            FetchRewrite::KeyRenamer(renamer) => renamer.apply(possible_types, value),
            FetchRewrite::ValueSetter(setter) => setter.apply(possible_types, value),
        }
    }
    fn apply_path(&self, possible_types: &PossibleTypes, value: &mut Value, path: &[FetchNodePathSegment]) {
        match self {
            FetchRewrite::KeyRenamer(renamer) => renamer.apply_path(possible_types, value, path),
            FetchRewrite::ValueSetter(setter) => setter.apply_path(possible_types, value, path),
        }
    }
}

impl ApplyFetchRewrite for KeyRenamer {
    fn apply(&self, possible_types: &PossibleTypes, value: &mut Value) {
        self.apply_path(possible_types, value, &self.path)
    }
    // Applies key rename operation on a Value (mutably)
    fn apply_path(&self, possible_types: &PossibleTypes, value: &mut Value, path: &[FetchNodePathSegment]) {
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

impl ApplyFetchRewrite for ValueSetter {
    fn apply(&self, possible_types: &PossibleTypes, data: &mut Value) {
        self.apply_path(possible_types, data, &self.path)
    }

    // Applies value setting on a Value (returns a new Value)
    fn apply_path(&self, possible_types: &PossibleTypes, data: &mut Value, path: &[FetchNodePathSegment]) {
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
                        if possible_types.entity_satisfies_type_condition(type_name, type_condition) {
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

#[instrument(level = "debug", skip_all, name = "process_root_result", fields(
    fetch_result = ?fetch_result.data.as_ref().map(|d| d.to_string()),
    errors_count = %fetch_result.errors.as_ref().map_or(0, |e| e.len()),
))]
fn process_root_result(
    fetch_result: ExecutionResult,
    execution_context: &mut QueryPlanExecutionContext<'_>,
    data: &mut Value,
) {
    // 4. Process the response
    if let Some(new_data) = fetch_result.data {
        if data.is_null() {
            *data = new_data; // Initialize with new_data
        } else {
            deep_merge::deep_merge(data, new_data);
        }
    }

    process_errors_and_extensions(
        execution_context,
        fetch_result.errors,
        fetch_result.extensions,
    );
}

enum ParallelJob<'a> {
    Root(ExecutionResult),
    Flatten((ExecuteForRepresentationsResult, Vec<&'a str>)),
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
                match node {
                    PlanNode::Fetch(fetch_node) => {
                        let job = fetch_node.execute_for_root(execution_context);
                        jobs.push(Box::pin(job.map(ParallelJob::Root)));
                    }
                    PlanNode::Flatten(flatten_node) => {
                        let normalized_path: Vec<&str> =
                            flatten_node.path.iter().map(String::as_str).collect();
                        let mut filtered_representations = String::with_capacity(1024);
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
                        let requires_nodes = fetch_node.requires.as_ref().unwrap();
                        let mut index = 0;
                        let mut indexes = BTreeSet::new();
                        filtered_representations.push('[');
                        traverse_and_callback(data, &normalized_path, &mut |entity| {
                            let is_projected =
                                if let Some(input_rewrites) = &fetch_node.input_rewrites {
                                    // We need to own the value and not modify the original entity
                                    let mut entity_owned = entity.to_owned();
                                    for input_rewrite in input_rewrites {
                                        input_rewrite.apply(
                                            &execution_context.schema_metadata.possible_types,
                                            &mut entity_owned,
                                        );
                                    }
                                    execution_context.project_requires(
                                        &requires_nodes.items,
                                        &entity_owned,
                                        &mut filtered_representations,
                                        indexes.is_empty(),
                                        None,
                                    )
                                } else {
                                    execution_context.project_requires(
                                        &requires_nodes.items,
                                        entity,
                                        &mut filtered_representations,
                                        indexes.is_empty(),
                                        None,
                                    )
                                };
                            if is_projected {
                                indexes.insert(index);
                            }
                            index += 1;
                        });
                        filtered_representations.push(']');
                        let job = fetch_node.execute_for_projected_representations(
                            execution_context,
                            filtered_representations,
                            indexes,
                        );
                        jobs.push(Box::pin(
                            job.map(|r| ParallelJob::Flatten((r, normalized_path))),
                        ));
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
                        if let Some(new_data) = fetch_result.data {
                            if data.is_null() {
                                *data = new_data; // Initialize with new_data
                            } else {
                                deep_merge::deep_merge(data, new_data);
                            }
                        }
                        // Process errors and extensions
                        if let Some(errors) = fetch_result.errors {
                            all_errors.extend(errors);
                        }
                        if let Some(extensions) = fetch_result.extensions {
                            all_extensions.push(extensions);
                        }
                    }
                    ParallelJob::Flatten((result, path)) => {
                        if let Some(mut entities) = result.entities {
                            let mut index_of_traverse = 0;
                            let mut index_of_entities = 0;
                            traverse_and_callback(data, &path, &mut |target| {
                                if result.indexes.contains(&index_of_traverse) {
                                    let entity =
                                        entities.get_mut(index_of_entities).unwrap().take();
                                    // Merge the entity into the target
                                    deep_merge::deep_merge(target, entity);
                                    index_of_entities += 1;
                                }
                                index_of_traverse += 1;
                            });
                        }
                        // Process errors and extensions
                        if let Some(errors) = result.errors {
                            all_errors.extend(errors);
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
        if !all_errors.is_empty() {
            execution_context.errors.extend(all_errors);
        }
        if !all_extensions.is_empty() {
            for extensions in all_extensions {
                execution_context.extensions.extend(extensions);
            }
        }
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
        let normalized_path: Vec<&str> = self.path.iter().map(String::as_str).collect();
        let now = std::time::Instant::now();
        let mut representations = vec![];
        let mut filtered_representations = String::with_capacity(1024);
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
        let requires_nodes = fetch_node.requires.as_ref().unwrap();
        filtered_representations.push('[');
        let mut first = true;
        traverse_and_callback(data, &normalized_path, &mut |entity| {
            let is_projected = if let Some(input_rewrites) = &fetch_node.input_rewrites {
                // We need to own the value and not modify the original entity
                let mut entity_owned = entity.to_owned();
                for input_rewrite in input_rewrites {
                    input_rewrite.apply(
                        &execution_context.schema_metadata.possible_types,
                        &mut entity_owned,
                    );
                }
                execution_context.project_requires(
                    &requires_nodes.items,
                    &entity_owned,
                    &mut filtered_representations,
                    first,
                    None,
                )
            } else {
                execution_context.project_requires(
                    &requires_nodes.items,
                    entity,
                    &mut filtered_representations,
                    first,
                    None,
                )
            };
            if is_projected {
                representations.push(entity);
                first = false;
            }
        });
        filtered_representations.push(']');
        trace!(
            "traversed and collected representations: {:?} in {:#?}",
            representations.len(),
            now.elapsed()
        );
        if first {
            // No representations collected, so we skip the fetch execution
            return;
        }
        let result = fetch_node
            .execute_for_projected_representations(
                execution_context,
                filtered_representations,
                BTreeSet::new(),
            )
            .await;
        if let Some(entities) = result.entities {
            for (entity, target) in entities.into_iter().zip(representations.iter_mut()) {
                // Merge the entity into the representation
                deep_merge::deep_merge(target, entity);
            }
        }

        process_errors_and_extensions(execution_context, result.errors, result.extensions);
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
        let condition_value = execution_context
            .variable_values
            .as_ref()
            .and_then(|vars| vars.get(&self.condition))
            .is_some_and(|val| match val {
                Value::Bool(b) => *b,
                _ => false,
            });
        if condition_value {
            if let Some(if_clause) = &self.if_clause {
                return if_clause.execute(execution_context, data).await;
            }
        } else if let Some(else_clause) = &self.else_clause {
            return else_clause.execute(execution_context, data).await;
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

impl ExecutionResult {
    pub fn from_error_message(message: String) -> ExecutionResult {
        ExecutionResult {
            data: None,
            errors: Some(vec![GraphQLError {
                message,
                locations: None,
                path: None,
                extensions: None,
            }]),
            extensions: None,
        }
    }
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
    pub representations: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResultExtensions {
    code: Option<String>,
    http: Option<HTTPErrorExtensions>,
    service_name: Option<String>,
    #[serde(flatten)]
    extensions: Option<HashMap<String, Value>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HTTPErrorExtensions {
    status: Option<u16>,
    headers: Option<HashMap<String, String>>,
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
        buffer: &mut String,
        first: bool,
        response_key: Option<&str>,
    ) -> bool {
        match entity {
            Value::Null => {
                return false;
            }
            Value::Bool(b) => {
                if !first {
                    buffer.push(',');
                }
                if let Some(response_key) = response_key {
                    buffer.push('"');
                    buffer.push_str(response_key);
                    buffer.push('"');
                    buffer.push(':');
                    buffer.push_str(if *b { "true" } else { "false" });
                } else {
                    buffer.push_str(if *b { "true" } else { "false" });
                }
            }
            Value::Number(n) => {
                if !first {
                    buffer.push(',');
                }
                if let Some(response_key) = response_key {
                    buffer.push('"');
                    buffer.push_str(response_key);
                    buffer.push_str("\":");
                }

                write!(buffer, "{}", n).unwrap()
            }
            Value::String(s) => {
                if !first {
                    buffer.push(',');
                }
                if let Some(response_key) = response_key {
                    buffer.push('"');
                    buffer.push_str(response_key);
                    buffer.push_str("\":");
                }
                write_and_escape_string(buffer, s);
            }
            Value::Array(entity_array) => {
                if !first {
                    buffer.push(',');
                }
                if let Some(response_key) = response_key {
                    buffer.push('"');
                    buffer.push_str(response_key);
                    buffer.push_str("\":[");
                } else {
                    buffer.push('[');
                }
                let mut first = true;
                for entity_item in entity_array {
                    let projected = self.project_requires(
                        requires_selections,
                        entity_item,
                        buffer,
                        first,
                        None,
                    );
                    if projected {
                        // Only update `first` if we actually write something
                        first = false;
                    }
                }
                buffer.push(']');
            }
            Value::Object(entity_obj) => {
                if requires_selections.is_empty() {
                    // It is probably a scalar with an object value, so we write it directly
                    buffer.push_str(&serde_json::to_string(entity_obj).unwrap());
                    return true;
                }
                if entity_obj.is_empty() {
                    return false;
                }

                let parent_first = first;
                let mut first = true;
                self.project_requires_map_mut(
                    requires_selections,
                    entity_obj,
                    buffer,
                    &mut first,
                    response_key,
                    parent_first,
                );
                if first {
                    // If no fields were projected, "first" is still true,
                    // so we skip writing the closing brace
                    return false;
                } else {
                    buffer.push('}');
                }
            }
        };
        true
    }

    fn project_requires_map_mut(
        &self,
        requires_selections: &Vec<SelectionItem>,
        entity_obj: &Map<String, Value>,
        buffer: &mut String,
        first: &mut bool,
        parent_response_key: Option<&str>,
        parent_first: bool,
    ) {
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
                            buffer.push(',');
                        }
                        if let Some(parent_response_key) = parent_response_key {
                            buffer.push('"');
                            buffer.push_str(parent_response_key);
                            buffer.push_str("\":");
                        }
                        buffer.push('{');
                        // Write __typename only if the object has other fields
                        if let Some(Value::String(type_name)) = entity_obj.get(TYPENAME_FIELD) {
                            buffer.push_str("\"__typename\":");
                            write_and_escape_string(buffer, type_name);
                            buffer.push(',');
                        }
                    }

                    // To avoid writing empty fields, we write to a temporary buffer first
                    self.project_requires(
                        &requires_selection.selections.items,
                        original,
                        buffer,
                        *first,
                        Some(response_key),
                    );
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
                            buffer,
                            first,
                            parent_response_key,
                            parent_first,
                        );
                    }
                }
                SelectionItem::FragmentSpread(_name_ref) => {
                    // We only minify the queries to subgraphs, so we never have fragment spreads here
                    unreachable!("Fragment spreads should not exist in FetchNode::requires.");
                }
            }
        }
    }
}

pub fn traverse_and_callback<'a, Callback>(
    current_data: &'a mut Value,
    remaining_path: &[&str],
    callback: &mut Callback,
) where
    Callback: FnMut(&'a mut Value),
{
    if remaining_path.is_empty() {
        if let Value::Array(arr) = current_data {
            // If the path is empty, we call the callback on each item in the array
            // We iterate because we want the entity objects directly
            for item in arr.iter_mut() {
                callback(item);
            }
        } else {
            // If the path is empty and current_data is not an array, just call the callback
            callback(current_data);
        }
        return;
    }

    let key = remaining_path[0];
    let rest_of_path = &remaining_path[1..];

    if key == "@" {
        if let Value::Array(list) = current_data {
            for item in list.iter_mut() {
                traverse_and_callback(item, rest_of_path, callback);
            }
        }
    } else if let Value::Object(map) = current_data {
        if let Some(next_data) = map.get_mut(key) {
            traverse_and_callback(next_data, rest_of_path, callback);
        }
    }
}

#[instrument(
    level = "trace",
    skip_all,
    fields(
        query_plan = ?query_plan,
        variable_values = ?variable_values,
        operation = ?operation.to_string(),
    )
)]
pub async fn execute_query_plan(
    query_plan: &QueryPlan,
    subgraph_executor_map: &SubgraphExecutorMap,
    variable_values: &Option<HashMap<String, Value>>,
    schema_metadata: &SchemaMetadata,
    operation: &OperationDefinition,
    has_introspection: bool,
    expose_query_plan: bool,
) -> String {
    let mut result_data = Value::Null; // Initialize data as Null
    let mut result_errors = vec![]; // Initial errors are empty
    #[allow(unused_mut)]
    let mut result_extensions = HashMap::new(); // Initial extensions are empty
    let mut execution_context = QueryPlanExecutionContext {
        variable_values,
        subgraph_executor_map,
        schema_metadata,
        errors: result_errors,
        extensions: result_extensions,
    };
    query_plan
        .execute(&mut execution_context, &mut result_data)
        .await;
    result_errors = execution_context.errors; // Get the final errors from the execution context
    result_extensions = execution_context.extensions; // Get the final extensions from the execution context
    if result_data.is_null() && has_introspection {
        result_data = Value::Object(Map::new()); // Ensure data is an empty object if it was null
    }
    if expose_query_plan {
        result_extensions.insert(
            "queryPlan".to_string(),
            serde_json::to_value(query_plan).unwrap(),
        );
    }
    projection::project_by_operation(
        &mut result_data,
        &mut result_errors,
        &result_extensions,
        operation,
        schema_metadata,
        variable_values,
    )
}

#[cfg(test)]
mod tests;
