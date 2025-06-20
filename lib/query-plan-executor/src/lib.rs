use async_trait::async_trait;
use query_planner::{
    ast::{
        operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
    },
    planner::plan_nodes::{
        ConditionNode, FetchNode, FetchRewrite, FlattenNode, KeyRenamer, ParallelNode, PlanNode,
        QueryPlan, SequenceNode, ValueSetter,
    },
    state::supergraph_state::OperationKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{collections::HashMap, vec};
use tracing::{instrument, trace, warn}; // For reading file in main

use crate::{
    deep_merge::deep_merge_objects, executors::map::SubgraphExecutorMap,
    schema_metadata::SchemaMetadata,
};
pub mod deep_merge;
pub mod executors;
pub mod introspection;
pub mod schema_metadata;
pub mod validation;
mod value_from_ast;
pub mod variables;

const TYPENAME_FIELD: &str = "__typename";
struct NodeResult {
    data: Value,                                // The data to put into the JSON structure
    json_path: String,                          // Path to the node in the JSON structure
    _errors: Option<Vec<GraphQLError>>,          // Errors that occurred during execution
    _extensions: Option<HashMap<String, Value>>, // Extensions to include in the result
}

impl NodeResult {
    fn apply(self, final_data: &mut Value) {
        if self.json_path == "/" {
            // If the path is root, we can directly set the data
            deep_merge::deep_merge(final_data, self.data);
            return;
        }
        let current_data = final_data.pointer_mut(&self.json_path);
        if let Some(data) = current_data {
            // Merge the data into the existing data at the path
            deep_merge::deep_merge(data, self.data);
        } else {
            println!(
                "NodeResult::apply: No data found at path {}",
                self.json_path
            );
        }
    }
}

#[async_trait]
trait ExecutablePlanNode {
    async fn execute_mut(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    );
    async fn execute_immut(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        data: &Value,
    ) -> Vec<NodeResult>;
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
    async fn execute_mut(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        match self {
            PlanNode::Fetch(node) => node.execute_mut(execution_context, data).await,
            PlanNode::Sequence(node) => node.execute_mut(execution_context, data).await,
            PlanNode::Parallel(node) => node.execute_mut(execution_context, data).await,
            PlanNode::Flatten(node) => node.execute_mut(execution_context, data).await,
            PlanNode::Condition(node) => node.execute_mut(execution_context, data).await,
            PlanNode::Subscription(node) => {
                // Subscriptions typically use a different protocol.
                // Execute the primary node for now.
                warn!(
            "Executing SubscriptionNode's primary as a normal node. Real subscription handling requires a different mechanism."
        );
                node.primary.execute_mut(execution_context, data).await
            }
            PlanNode::Defer(_) => {
                // Defer/Deferred execution is complex.
                warn!("DeferNode execution is not fully implemented.");
            }
        }
    }
    async fn execute_immut(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        data: &Value,
    ) -> Vec<NodeResult> {
        match self {
            PlanNode::Fetch(node) => node.execute_immut(execution_context, data).await,
            PlanNode::Sequence(node) => node.execute_immut(execution_context, data).await,
            PlanNode::Parallel(node) => node.execute_immut(execution_context, data).await,
            PlanNode::Flatten(node) => node.execute_immut(execution_context, data).await,
            PlanNode::Condition(node) => node.execute_immut(execution_context, data).await,
            PlanNode::Subscription(node) => {
                // Subscriptions typically use a different protocol.
                // Execute the primary node for now.
                warn!(
            "Executing SubscriptionNode's primary as a normal node. Real subscription handling requires a different mechanism."
        );
                node.primary.execute_immut(execution_context, data).await
            }
            PlanNode::Defer(_) => {
                // Defer/Deferred execution is complex.
                warn!("DeferNode execution is not fully implemented.");
                vec![]
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

#[instrument(
    level = "debug", 
    skip_all
    name = "process_representations_result",
    fields(
        representations_count = %result.entities.as_ref().map_or(0, |e| e.len())
    ),
)]
fn process_representations_result(
    result: ExecuteForRepresentationsResult,
    indexes: Vec<usize>,
    representations: &mut Vec<&mut Value>,
    execution_context: &mut QueryPlanExecutionContext<'_>,
) {
    if let Some(entities) = result.entities {
        trace!(
            "Processing representations result: {} entities",
            entities.len()
        );
        for (entity, index) in entities.into_iter().zip(indexes.into_iter()) {
            if let Some(representation) = representations.get_mut(index) {
                trace!(
                    "Merging entity into representation at index {}: {:?}",
                    index,
                    entity
                );
                deep_merge::deep_merge(representation, entity);
            }
        }
    }
    process_errors_and_extensions(execution_context, result.errors, result.extensions);
}

struct ExecuteForRepresentationsResult {
    entities: Option<Vec<Value>>,
    errors: Option<Vec<GraphQLError>>,
    extensions: Option<HashMap<String, Value>>,
}

#[async_trait]
trait ExecutableFetchNode {
    async fn execute_for_root(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
    ) -> NodeResult;
    fn project_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        representations: &[&mut Value],
    ) -> ProjectRepresentationsResult;
    async fn execute_for_projected_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        filtered_representations: Vec<Value>,
    ) -> ExecuteForRepresentationsResult;
    fn apply_output_rewrites(
        &self,
        possible_types: &HashMap<String, Vec<String>>,
        data: &mut Value,
    );
    fn prepare_variables_for_fetch_node(
        &self,
        variable_values: &Option<HashMap<String, Value>>,
    ) -> Option<HashMap<String, Value>>;
}

#[async_trait]
impl ExecutablePlanNode for FetchNode {
    #[instrument(level = "debug", skip_all, name = "FetchNode::execute")]
    async fn execute_immut(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        _data: &Value,
    ) -> Vec<NodeResult> {
        let result = self.execute_for_root(execution_context).await;
        vec![result]
    }
    async fn execute_mut(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        let result = self.execute_for_root(execution_context).await;
        // Apply the result to the data
        result.apply(data);
    }
}

struct ProjectRepresentationsResult {
    representations: Vec<Value>,
    indexes: Vec<usize>,
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
            operation_str = %self.operation.operation_str,
        )
    )]
    async fn execute_for_root(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
    ) -> NodeResult {
        let variables = self.prepare_variables_for_fetch_node(execution_context.variable_values);

        let execution_request = ExecutionRequest {
            query: self.operation.operation_str.clone(),
            operation_name: self.operation_name.clone(),
            variables,
            extensions: None,
        };
        let mut fetch_result = execution_context
            .subgraph_executor_map
            .execute(&self.service_name, execution_request)
            .await;

        // 5. Process the response

        if let Some(new_data) = &mut fetch_result.data {
            self.apply_output_rewrites(&execution_context.schema_metadata.possible_types, new_data);
        }

        NodeResult {
            data: fetch_result.data.unwrap_or(Value::Null),
            json_path: "/".to_string(), // Root path
            errors: fetch_result.errors,
            extensions: fetch_result.extensions,
        }
    }

    fn project_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        representations: &[&mut Value],
    ) -> ProjectRepresentationsResult {
        let mut filtered_repr_indexes = Vec::new();
        // 1. Filter representations based on requires (if present)
        let mut filtered_representations: Vec<Value> = Vec::new();
        let requires_nodes = self.requires.as_ref().unwrap();
        for (index, entity) in representations.iter().enumerate() {
            let entity_projected =
                execution_context.project_requires(&requires_nodes.items, entity);
            if !entity_projected.is_null() {
                filtered_representations.push(entity_projected);
                filtered_repr_indexes.push(index);
            }
        }

        if let Some(input_rewrites) = &self.input_rewrites {
            for representation in filtered_representations.iter_mut() {
                for rewrite in input_rewrites {
                    rewrite.apply(
                        &execution_context.schema_metadata.possible_types,
                        representation,
                    );
                }
            }
        }

        ProjectRepresentationsResult {
            representations: filtered_representations,
            indexes: filtered_repr_indexes,
        }
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
        filtered_representations: Vec<Value>,
    ) -> ExecuteForRepresentationsResult {
        // 2. Prepare variables for fetch
        let mut variables = self
            .prepare_variables_for_fetch_node(execution_context.variable_values)
            .unwrap_or_default();
        variables.insert(
            "representations".to_string(),
            Value::Array(filtered_representations),
        );

        let execution_request = ExecutionRequest {
            query: self.operation.operation_str.clone(),
            operation_name: self.operation_name.clone(),
            variables: Some(variables),
            extensions: None,
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
            errors: fetch_result.errors,
            extensions: fetch_result.extensions,
        }
    }

    fn apply_output_rewrites(
        &self,
        possible_types: &HashMap<String, Vec<String>>,
        data: &mut Value,
    ) {
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
    fn prepare_variables_for_fetch_node(
        &self,
        variable_values: &Option<HashMap<String, Value>>,
    ) -> Option<HashMap<String, Value>> {
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
                                    .map(|v| (variable_name.to_string(), v.clone()))
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
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, value: &mut Value);
    fn apply_path(
        &self,
        possible_types: &HashMap<String, Vec<String>>,
        value: &mut Value,
        path: &[String],
    );
}

impl ApplyFetchRewrite for FetchRewrite {
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, value: &mut Value) {
        match self {
            FetchRewrite::KeyRenamer(renamer) => renamer.apply(possible_types, value),
            FetchRewrite::ValueSetter(setter) => setter.apply(possible_types, value),
        }
    }
    fn apply_path(
        &self,
        possible_types: &HashMap<String, Vec<String>>,
        value: &mut Value,
        path: &[String],
    ) {
        match self {
            FetchRewrite::KeyRenamer(renamer) => renamer.apply_path(possible_types, value, path),
            FetchRewrite::ValueSetter(setter) => setter.apply_path(possible_types, value, path),
        }
    }
}

impl ApplyFetchRewrite for KeyRenamer {
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, value: &mut Value) {
        self.apply_path(possible_types, value, &self.path)
    }
    // Applies key rename operation on a Value (mutably)
    fn apply_path(
        &self,
        possible_types: &HashMap<String, Vec<String>>,
        value: &mut Value,
        path: &[String],
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
                let type_condition = current_segment.strip_prefix("... on ");
                match type_condition {
                    Some(type_condition) => {
                        let type_name = match obj.get(TYPENAME_FIELD) {
                            Some(Value::String(type_name)) => type_name,
                            _ => type_condition, // Default to type_condition if not found
                        };
                        if entity_satisfies_type_condition(
                            possible_types,
                            type_name,
                            type_condition,
                        ) {
                            self.apply_path(possible_types, value, remaining_path)
                        }
                    }
                    _ => {
                        if remaining_path.is_empty() {
                            if *current_segment != self.rename_key_to {
                                if let Some(val) = obj.remove(current_segment) {
                                    obj.insert(self.rename_key_to.to_string(), val);
                                }
                            }
                        } else if let Some(next_value) = obj.get_mut(current_segment) {
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
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, data: &mut Value) {
        self.apply_path(possible_types, data, &self.path)
    }

    // Applies value setting on a Value (returns a new Value)
    fn apply_path(
        &self,
        possible_types: &HashMap<String, Vec<String>>,
        data: &mut Value,
        path: &[String],
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
                let current_key = &path[0];
                let remaining_path = &path[1..];

                if let Some(type_condition) = current_key.strip_prefix("... on ") {
                    let type_name = match map.get(TYPENAME_FIELD) {
                        Some(Value::String(type_name)) => type_name,
                        _ => type_condition, // Default to type_condition if not found
                    };
                    if entity_satisfies_type_condition(possible_types, type_name, type_condition) {
                        self.apply_path(possible_types, data, remaining_path)
                    }
                } else if let Some(data) = map.get_mut(current_key) {
                    // If the key exists, apply the remaining path to its value
                    self.apply_path(possible_types, data, remaining_path)
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
    async fn execute_mut(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        for node in &self.nodes {
            node.execute_mut(execution_context, data) // No representations passed to child nodes
                .await;
        }
    }
    async fn execute_immut(
        &self,
        _execution_context: &QueryPlanExecutionContext<'_>,
        _data: &Value,
    ) -> Vec<NodeResult> {
        unimplemented!("...")
    }
}

#[async_trait]
impl ExecutablePlanNode for ParallelNode {
    #[instrument(level = "trace", skip_all, name = "ParallelNode::execute", fields(
        nodes_count = %self.nodes.len()
    ))]
    async fn execute_immut(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        data: &Value,
    ) -> Vec<NodeResult> {
        let mut jobs = vec![];
        for node in &self.nodes {
            let job = node.execute_immut(execution_context, data);
            jobs.push(job);
        }
        let results = futures::future::join_all(jobs).await;
        results.into_iter().flatten().collect()
    }
    async fn execute_mut(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        let mut jobs = vec![];
        for node in &self.nodes {
            let job = node.execute_immut(execution_context, data);
            jobs.push(job);
        }
        let results = futures::future::join_all(jobs).await;
        for result in results {
            for node_result in result {
                node_result.apply(data);
            }
        }
    }
}

struct CollectedRepresentation {
    representation: Value, // The representation data
    json_path: String,     // The path to the representation in the JSON structure
}


fn collect_paths(
    root_data: &Value,
    base_path: &str,
    remaining_path: &[&str],
) -> Vec<String> {
    // If @, handle arrays
    if remaining_path.is_empty() {
        return vec![base_path.to_string()];
    }
    let current_segment = remaining_path[0];
    let next_path = &remaining_path[1..];
    match (current_segment, root_data) {
        ("@", Value::Array(arr)) => {
            arr.iter().enumerate().flat_map(|(index, item)| {
                let new_path = format!("{}/{}", base_path, index);
                collect_paths(
                    item,
                    &new_path,
                    next_path,
                )
            }).collect()
        },
        (_, Value::Object(obj)) => {
            // If current segment is a key in the object
            if let Some(value) = obj.get(current_segment) {
                let new_path = if base_path == "/" {
                    format!("/{}", current_segment)
                } else {
                    format!("{}/{}", base_path, current_segment)
                };
                collect_paths(
                    value,
                    &new_path,
                    next_path,
                )
            } else {
                // If the key does not exist, return empty paths
                vec![]
            }
        }
        _ => {
            // If no match, return empty paths
            vec![]
        }
    }
}

fn collect_representations(
    root_data: &Value,
    base_path: &str,
    remaining_path: &[&str],
    execution_context: &QueryPlanExecutionContext<'_>,
    requires_items: &Vec<SelectionItem>,
) -> Vec<CollectedRepresentation> {
    // If @, handle arrays
    if remaining_path.is_empty() {
        let projected = execution_context.project_requires(requires_items, root_data);
        return if projected.is_null() {
            vec![]
        } else {
            vec![CollectedRepresentation {
                representation: projected,
                json_path: base_path.to_string(),
            }]
        };
    }
    let current_segment = remaining_path[0];
    let next_path = &remaining_path[1..];
    match (current_segment, root_data) {
        ("@", Value::Array(arr)) => {
            arr.iter().enumerate().flat_map(|(index, item)| {
                let new_path = format!("{}/{}", base_path, index);
                collect_representations(
                    item,
                    &new_path,
                    next_path,
                    execution_context,
                    requires_items,
                )
            }).collect()
        },
        (_, Value::Object(obj)) => {
            // If current segment is a key in the object
            if let Some(value) = obj.get(current_segment) {
                let new_path = if base_path == "/" {
                    format!("/{}", current_segment)
                } else {
                    format!("{}/{}", base_path, current_segment)
                };
                collect_representations(
                    value,
                    &new_path,
                    next_path,
                    execution_context,
                    requires_items,
                )
            } else {
                // If the key does not exist, return empty paths
                vec![]
            }
        }
        _ => {
            // If no match, return empty paths
            vec![]
        }
    }
}

#[async_trait]
impl ExecutablePlanNode for FlattenNode {
    async fn execute_immut(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        data: &Value,
    ) -> Vec<NodeResult> {
        let fetch_node = match self.node.as_ref() {
            PlanNode::Fetch(fetch_node) => fetch_node,
            _ => {
                unimplemented!(
                    "FlattenNode can only execute FetchNode as child node, found: {:?}",
                    self.node
                );
            }
        };
        let collected_representations = collect_representations(
            data,
            "/",
            &self.path.iter().map(String::as_str).collect::<Vec<_>>(),
            execution_context,
            &fetch_node.requires.as_ref().unwrap().items,
        );
        let mut filtered_representations = vec![];
        let mut json_paths = vec![];
        for collected_representation in collected_representations {
            filtered_representations.push(collected_representation.representation);
            json_paths.push(collected_representation.json_path);
        }
        let result = fetch_node
            .execute_for_projected_representations(execution_context, filtered_representations)
            .await;
        match result.entities {
            Some(entities) => entities
                .into_iter()
                .zip(json_paths.into_iter())
                .map(|(data, json_path)| NodeResult {
                    data,
                    json_path,
                    errors: result.errors.clone(),
                    extensions: result.extensions.clone(),
                })
                .collect(),
            None => vec![],
        }
    }

    #[instrument(level = "trace", skip_all, name = "FlattenNode::execute", fields(
        path = ?self.path
    ))]
    async fn execute_mut(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        // Execute the child node. `execution_context` can be borrowed mutably
        // because `collected_representations` borrows `data_for_flatten`, not `execution_context.data`.
        let normalized_path: Vec<&str> = self.path.iter().map(String::as_str).collect();
        let now = std::time::Instant::now();
        let mut representations = traverse_and_collect(data, normalized_path.as_slice());
        trace!(
            "traversed and collected representations: {:?} in {:#?}",
            representations.len(),
            now.elapsed()
        );
        match self.node.as_ref() {
            PlanNode::Fetch(fetch_node) => {
                let now = std::time::Instant::now();
                let ProjectRepresentationsResult {
                    representations: filtered_representations,
                    indexes: filtered_repr_indexes,
                } = fetch_node.project_representations(execution_context, &representations);
                trace!(
                    "projected representations: {:?} in {:#?}",
                    representations.len(),
                    now.elapsed()
                );

                let now = std::time::Instant::now();
                let result = fetch_node
                    .execute_for_projected_representations(
                        execution_context,
                        filtered_representations,
                    )
                    .await;
                trace!(
                    "executed projected representations: {:?} in {:?}",
                    representations.len(),
                    now.elapsed()
                );
                // Process the result
                process_representations_result(
                    result,
                    filtered_repr_indexes,
                    &mut representations,
                    execution_context,
                );
                trace!(
                    "processed projected representations: {:?} in {:?}",
                    representations.len(),
                    now.elapsed()
                );
            }
            _ => {
                unimplemented!(
                    "FlattenNode can only execute FetchNode as child node, found: {:?}",
                    self.node
                );
            }
        }
    }
}

trait InnerNode {
    fn get_inner_node<'a>(
        &'a self,
        variable_values: &Option<HashMap<String, Value>>,
    ) -> Option<&'a Box<PlanNode>>;
}

impl InnerNode for ConditionNode {
    fn get_inner_node<'a>(
        &'a self,
        variable_values: &Option<HashMap<String, Value>>,
    ) -> Option<&'a Box<PlanNode>> {
        // Get the condition variable from the context
        let condition_value: bool = match variable_values {
            Some(ref variable_values) => {
                match variable_values.get(&self.condition) {
                    Some(value) => {
                        // Check if the value is a boolean
                        match value {
                            Value::Bool(b) => *b,
                            _ => true, // Default to true if not a boolean
                        }
                    }
                    None => {
                        // If the variable is not found, default to false
                        false
                    }
                }
            }
            None => {
                // No variable values provided, default to false
                false
            }
        };
        if condition_value {
            if let Some(if_clause) = &self.if_clause {
                return Some(if_clause);
            }
        } else if let Some(else_clause) = &self.else_clause {
            return Some(else_clause);
        }
        None
    }
}

#[async_trait]
impl ExecutablePlanNode for ConditionNode {
    #[instrument(level = "trace", skip_all, name = "ConditionNode::execute")]
    async fn execute_mut(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        let inner_node = self.get_inner_node(execution_context.variable_values);
        if let Some(node) = inner_node {
            // Execute the inner node if the condition is met
            node.execute_mut(execution_context, data).await;
        } else {
            // If no inner node is found, do nothing
            trace!("ConditionNode condition not met, skipping execution.");
        }
    }
    async fn execute_immut(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        data: &Value,
    ) -> Vec<NodeResult> {
        let inner_node = self.get_inner_node(execution_context.variable_values);
        if let Some(node) = inner_node {
            // Execute the inner node if the condition is met
            node.execute_immut(execution_context, data).await
        } else {
            // If no inner node is found, do nothing
            trace!("ConditionNode condition not met, skipping execution.");
            vec![]
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
            root_node.execute_mut(execution_context, data).await
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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, Value>>,
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
    #[instrument(
        level = "trace",
        skip_all,
        fields(
            requires_selections = ?requires_selections.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            entity = ?entity
        )
    )]
    pub fn project_requires(
        &self,
        requires_selections: &Vec<SelectionItem>,
        entity: &Value,
    ) -> Value {
        if requires_selections.is_empty() {
            return entity.clone(); // No selections to project, return the entity as is
        }
        match entity {
            Value::Null => Value::Null,
            Value::Array(entity_array) => Value::Array(
                entity_array
                    .iter()
                    .map(|item| self.project_requires(requires_selections, item))
                    .collect(),
            ),
            Value::Object(entity_obj) => {
                let mut result_map = Map::new();
                for requires_selection in requires_selections {
                    match &requires_selection {
                        SelectionItem::Field(requires_selection) => {
                            let field_name = &requires_selection.name;
                            let response_key = requires_selection.selection_identifier();
                            let original = entity_obj
                                .get(field_name)
                                .unwrap_or(entity_obj.get(response_key).unwrap_or(&Value::Null));
                            let projected_value: Value = self
                                .project_requires(&requires_selection.selections.items, original);
                            if !projected_value.is_null() {
                                result_map.insert(response_key.to_string(), projected_value);
                            }
                        }
                        SelectionItem::InlineFragment(requires_selection) => {
                            let type_name = match entity_obj.get(TYPENAME_FIELD) {
                                Some(Value::String(type_name)) => type_name,
                                _ => requires_selection.type_condition.as_str(),
                            };
                            if entity_satisfies_type_condition(
                                &self.schema_metadata.possible_types,
                                type_name,
                                &requires_selection.type_condition,
                            ) {
                                let projected = self
                                    .project_requires(&requires_selection.selections.items, entity);
                                // Merge the projected value into the result
                                if let Value::Object(projected_map) = projected {
                                    deep_merge::deep_merge_objects(&mut result_map, projected_map);
                                    /*
                                     * TLDR: Needed for interface objects
                                     *
                                     * There are cases the type name in `__typename` might not exist in the subgraph.
                                     * We know that the type name in the type condition exists,
                                     * so we set the `__typename` field to the value from the type condition to guarantee
                                     * that the type name in `__typename` is always present in the result.
                                     */
                                    result_map.insert(
                                        TYPENAME_FIELD.to_string(),
                                        json!(requires_selection.type_condition),
                                    );
                                }
                                // If the projected value is not an object, it will be ignored
                            }
                        }
                    }
                }
                if (result_map.is_empty())
                    || (result_map.len() == 1 && result_map.contains_key(TYPENAME_FIELD))
                {
                    Value::Null
                } else {
                    Value::Object(result_map)
                }
            }
            Value::Bool(bool) => Value::Bool(*bool),
            Value::Number(num) => Value::Number(num.to_owned()),
            Value::String(string) => Value::String(string.to_string()),
        }
    }
}

#[instrument(
    level = "trace",
    skip_all,
    name = "entity_satisfies_type_condition",
    fields(
        type_name = %type_name,
        type_condition = %type_condition,
    )
)]
fn entity_satisfies_type_condition(
    possible_types: &HashMap<String, Vec<String>>,
    type_name: &str,
    type_condition: &str,
) -> bool {
    if type_name == type_condition {
        true
    } else {
        let possible_types_for_type_condition = possible_types.get(type_condition);
        match possible_types_for_type_condition {
            Some(possible_types_for_type_condition) => {
                possible_types_for_type_condition.contains(&type_name.to_string())
            }
            None => {
                // If no possible types are found, return false
                false
            }
        }
    }
}

// --- Helper Function for Flatten ---

/// Recursively traverses the data according to the path segments,
/// handling '@' for array iteration, and collects the final values.current_data.to_vec()
#[instrument(level = "trace", skip_all, fields(
    current_type = ?current_data,
    remaining_path = ?remaining_path
))]
pub fn traverse_and_collect<'a>(
    current_data: &'a mut Value,
    remaining_path: &[&str],
) -> Vec<&'a mut Value> {
    match (current_data, remaining_path) {
        (Value::Array(arr), []) => arr.iter_mut().collect(), // Base case: No more path segments, return all items in the array
        (current_data, []) => vec![current_data],            // Base case: No more path segments,
        (Value::Object(obj), [next_segment, next_remaining_path @ ..]) => {
            if let Some(next_value) = obj.get_mut(*next_segment) {
                traverse_and_collect(next_value, next_remaining_path)
            } else {
                vec![] // No valid path segment
            }
        }
        (Value::Array(arr), ["@", next_remaining_path @ ..]) => arr
            .iter_mut()
            .flat_map(|item| traverse_and_collect(item, next_remaining_path))
            .collect(),
        _ => vec![], // No valid path segment
    }
}

// --- Helper Functions ---

// --- Main Function (for testing) ---

#[instrument(
    level = "trace", 
    skip_all,
    fields(
        type_name = %type_name,
        selection_set = ?selection_set.items.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        obj = ?obj
    )
)]
fn project_selection_set_with_map(
    obj: &mut Map<String, Value>,
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
) -> Option<Map<String, Value>> {
    let type_name = match obj.get(TYPENAME_FIELD) {
        Some(Value::String(type_name)) => type_name,
        _ => type_name,
    }
    .to_string();
    let mut new_obj = Map::new();
    let field_map = schema_metadata.type_fields.get(&type_name);
    for selection in &selection_set.items {
        match selection {
            SelectionItem::Field(field) => {
                // Get the type fields for the current type
                // Type is not found in the schema
                if field_map.is_none() {
                    // It won't reach here already, as the selection should be validated before projection
                    warn!("Type {} not found. Skipping projection.", type_name);
                    continue;
                }
                if let Some(ref skip_variable) = field.skip_if {
                    let variable_value = variable_values
                        .as_ref()
                        .and_then(|vars| vars.get(skip_variable));
                    if variable_value == Some(&Value::Bool(true)) {
                        continue; // Skip this field if the variable is true
                    }
                }
                if let Some(ref include_variable) = field.include_if {
                    let variable_value = variable_values
                        .as_ref()
                        .and_then(|vars| vars.get(include_variable));
                    if variable_value != Some(&Value::Bool(true)) {
                        continue; // Skip this field if the variable is not true
                    }
                }
                let response_key = field.alias.as_ref().unwrap_or(&field.name).to_string();
                if field.name == TYPENAME_FIELD {
                    new_obj.insert(response_key, Value::String(type_name.to_string()));
                    continue;
                }
                let field_map = field_map.unwrap();
                let field_type = field_map.get(&field.name);
                if field.name == "__schema" && type_name == "Query" {
                    obj.insert(
                        response_key.to_string(),
                        schema_metadata.introspection_schema_root_json.clone(),
                    );
                }
                let field_val = obj.get_mut(&response_key);
                match (field_type, field_val) {
                    (Some(field_type), Some(field_val)) => {
                        match field_val {
                            Value::Object(field_val_map) => {
                                let new_field_val_map = project_selection_set_with_map(
                                    field_val_map,
                                    errors,
                                    &field.selections,
                                    field_type,
                                    schema_metadata,
                                    variable_values,
                                );
                                match new_field_val_map {
                                    Some(new_field_val_map) => {
                                        // If the field is an object, merge the projected values
                                        new_obj
                                            .insert(response_key, Value::Object(new_field_val_map));
                                    }
                                    None => {
                                        new_obj.insert(response_key, Value::Null);
                                    }
                                }
                            }
                            field_val => {
                                project_selection_set(
                                    field_val,
                                    errors,
                                    &field.selections,
                                    field_type,
                                    schema_metadata,
                                    variable_values,
                                );
                                let field_val = std::mem::take(field_val);
                                new_obj.insert(
                                    response_key,
                                    field_val, // Clone the value to insert
                                );
                            }
                        }
                    }
                    (Some(_field_type), None) => {
                        // If the field is not found in the object, set it to Null
                        new_obj.insert(response_key, Value::Null);
                    }
                    (None, _) => {
                        // It won't reach here already, as the selection should be validated before projection
                        warn!(
                            "Field {} not found in type {}. Skipping projection.",
                            field.name, type_name
                        );
                    }
                }
            }
            SelectionItem::InlineFragment(inline_fragment) => {
                if entity_satisfies_type_condition(
                    &schema_metadata.possible_types,
                    &type_name,
                    &inline_fragment.type_condition,
                ) {
                    let sub_new_obj = project_selection_set_with_map(
                        obj,
                        errors,
                        &inline_fragment.selections,
                        &type_name,
                        schema_metadata,
                        variable_values,
                    );
                    if let Some(sub_new_obj) = sub_new_obj {
                        // If the inline fragment projection returns a new object, merge it
                        deep_merge_objects(&mut new_obj, sub_new_obj)
                    } else {
                        // If the inline fragment projection returns None, skip it
                        continue;
                    }
                }
            }
        }
    }
    Some(new_obj)
}

#[instrument(
    level = "trace", 
    skip_all,
    fields(
        type_name = %type_name,
        selection_set = ?selection_set.items.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        data = ?data
    )
)]
fn project_selection_set(
    data: &mut Value,
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
) {
    match data {
        Value::Null => {
            // If data is Null, no need to project further
        }
        Value::String(value) => {
            if let Some(enum_values) = schema_metadata.enum_values.get(type_name) {
                if !enum_values.contains(value) {
                    // If the value is not a valid enum value, add an error
                    // and set data to Null
                    *data = Value::Null; // Set data to Null if the value is not valid
                    errors.push(GraphQLError {
                        message: format!(
                            "Value is not a valid enum value for type '{}'",
                            type_name
                        ),
                        locations: None,
                        path: None,
                        extensions: None,
                    });
                }
            } // No further processing needed for strings
        }
        Value::Array(arr) => {
            // If data is an array, project each item in the array
            for item in arr {
                project_selection_set(
                    item,
                    errors,
                    selection_set,
                    type_name,
                    schema_metadata,
                    variable_values,
                );
            } // No further processing needed for arrays
        }
        Value::Object(obj) => {
            match project_selection_set_with_map(
                obj,
                errors,
                selection_set,
                type_name,
                schema_metadata,
                variable_values,
            ) {
                Some(new_obj) => {
                    // If the projection returns a new object, replace the old one
                    *obj = new_obj;
                }
                None => {
                    // If the projection returns None, set data to Null
                    *data = Value::Null;
                }
            }
        }
        _ => {}
    }
}

#[instrument(level = "trace", skip_all)]
pub fn project_data_by_operation(
    data: &mut Value,
    errors: &mut Vec<GraphQLError>,
    operation: &OperationDefinition,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
) {
    let root_type_name = match operation.operation_kind {
        Some(OperationKind::Query) => "Query",
        Some(OperationKind::Mutation) => "Mutation",
        Some(OperationKind::Subscription) => "Subscription",
        None => "Query",
    };
    // Project the data based on the selection set
    project_selection_set(
        data,
        errors,
        &operation.selection_set,
        root_type_name,
        schema_metadata,
        variable_values,
    )
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
) -> ExecutionResult {
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
    if !result_data.is_null() || has_introspection {
        if result_data.is_null() {
            result_data = Value::Object(serde_json::Map::new()); // Initialize as empty object if Null
        }
        project_data_by_operation(
            &mut result_data,
            &mut result_errors,
            operation,
            schema_metadata,
            variable_values,
        );
    }

    ExecutionResult {
        data: if result_data.is_null() {
            None
        } else {
            Some(result_data)
        },
        errors: if result_errors.is_empty() {
            None
        } else {
            Some(result_errors)
        },
        extensions: if result_extensions.is_empty() {
            None
        } else {
            Some(result_extensions)
        },
    }
}

#[cfg(test)]
mod tests;
