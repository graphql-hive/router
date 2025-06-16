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
use serde_json::{Map, Value};
use std::{collections::HashMap, sync::Arc, vec};
use tracing::{debug, instrument, warn}; // For reading file in main

use crate::{deep_merge::deep_merge_objects, schema_metadata::SchemaMetadata};
mod deep_merge;
pub mod executors;
pub mod introspection;
pub mod schema_metadata;
pub mod validation;
mod value_from_ast;
pub mod variables;

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
    // 6. Handle errors
    if let Some(errors) = errors {
        execution_context.errors.extend(errors);
    }
    // 7. Handle extensions
    if let Some(extensions) = extensions {
        execution_context.extensions.extend(extensions);
    }
}

fn process_representations_result(
    result: ExecuteForRepresentationsResult,
    representations: &mut Vec<&mut Value>,
    execution_context: &mut QueryPlanExecutionContext<'_>,
) {
    if let Some(entities) = result.entities {
        // 3. Process the entities
        for (entity, index) in entities.into_iter().zip(result.indexes.into_iter()) {
            if let Some(representation) = representations.get_mut(index) {
                // Merge the entity into the representation
                deep_merge::deep_merge(representation, entity);
            }
        }
    }
    process_errors_and_extensions(execution_context, result.errors, result.extensions);
}

struct ExecuteForRepresentationsResult {
    entities: Option<Vec<Value>>,
    indexes: Vec<usize>,
    errors: Option<Vec<GraphQLError>>,
    extensions: Option<HashMap<String, Value>>,
}

#[async_trait]
trait ExecutableFetchNode {
    async fn execute_for_root(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
    ) -> ExecutionResult;
    fn project_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        representations: &[&mut Value],
    ) -> ProjectRepresentationsResult;
    async fn execute_for_projected_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        filtered_representations: Vec<Value>,
        filtered_repr_indexes: Vec<usize>,
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
    #[instrument(skip(self, execution_context), name = "FetchNode::execute")]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        let fetch_result = self.execute_for_root(execution_context).await;

        process_result(fetch_result, execution_context, data);
    }
}

struct ProjectRepresentationsResult {
    representations: Vec<Value>,
    indexes: Vec<usize>,
}

#[async_trait]
impl ExecutableFetchNode for FetchNode {
    async fn execute_for_root(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
    ) -> ExecutionResult {
        let variables = self.prepare_variables_for_fetch_node(execution_context.variable_values);

        let mut fetch_result = execution_context
            .execute(
                &self.service_name,
                ExecutionRequest {
                    query: self.operation.operation_str.clone(),
                    operation_name: self.operation_name.clone(),
                    variables,
                    extensions: None,
                },
            )
            .await;

        // 5. Process the response

        if let Some(new_data) = &mut fetch_result.data {
            self.apply_output_rewrites(&execution_context.schema_metadata.possible_types, new_data);
        }

        fetch_result
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

    async fn execute_for_projected_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        filtered_representations: Vec<Value>,
        filtered_repr_indexes: Vec<usize>,
    ) -> ExecuteForRepresentationsResult {
        // 2. Prepare variables for fetch
        let mut variables = self
            .prepare_variables_for_fetch_node(execution_context.variable_values)
            .unwrap_or_default();

        variables.insert(
            "representations".to_string(),
            Value::Array(filtered_representations),
        );

        // 3. Execute the fetch operation
        let fetch_result = execution_context
            .execute(
                &self.service_name,
                ExecutionRequest {
                    query: self.operation.operation_str.clone(),
                    operation_name: self.operation_name.clone(),
                    variables: Some(variables),
                    extensions: None,
                },
            )
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
            indexes: filtered_repr_indexes,
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
                        let type_name = match obj.get("__typename") {
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
                    let type_name = match map.get("__typename") {
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
    #[instrument(skip(self, execution_context), name = "SequenceNode::execute")]
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

fn process_result(
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

#[async_trait]
impl ExecutablePlanNode for ParallelNode {
    #[instrument(skip(self, execution_context), name = "ParallelNode::execute")]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        // Here we call fetch nodes in parallel and non-fetch nodes sequentially.
        let mut fetch_jobs = vec![];
        let mut flatten_jobs = vec![];
        let mut flatten_paths = vec![];

        // Collect Fetch node results and non-fetch nodes for sequential execution
        for node in &self.nodes {
            match node {
                PlanNode::Fetch(fetch_node) => {
                    // Execute FetchNode in parallel
                    let job = fetch_node.execute_for_root(execution_context);
                    fetch_jobs.push(job);
                }
                PlanNode::Flatten(flatten_node) => {
                    let normalized_path: Vec<&str> =
                        flatten_node.path.iter().map(String::as_str).collect();
                    let collected_representations = traverse_and_collect(data, &normalized_path);
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
                    let project_result = fetch_node
                        .project_representations(execution_context, &collected_representations);
                    let job = fetch_node.execute_for_projected_representations(
                        execution_context,
                        project_result.representations,
                        project_result.indexes,
                    );
                    flatten_jobs.push(job);
                    flatten_paths.push(normalized_path);
                }
                _ => {}
            }
        }

        let mut all_errors = vec![];
        let mut all_extensions = vec![];
        let flatten_results = futures::future::join_all(flatten_jobs).await;
        for (result, path) in flatten_results.into_iter().zip(flatten_paths) {
            // Process FlattenNode results
            if let Some(entities) = result.entities {
                let mut collected_representations = traverse_and_collect(data, &path);
                for (entity, index) in entities.into_iter().zip(result.indexes.into_iter()) {
                    if let Some(representation) = collected_representations.get_mut(index) {
                        // Merge the entity into the representation
                        deep_merge::deep_merge(representation, entity);
                    }
                }
            }
            // Extend errors and extensions from the result
            if let Some(errors) = result.errors {
                all_errors.extend(errors);
            }
            if let Some(extensions) = result.extensions {
                all_extensions.push(extensions);
            }
        }

        let fetch_results = futures::future::join_all(fetch_jobs).await;

        // Process results from FetchNode executions
        for fetch_result in fetch_results {
            process_result(fetch_result, execution_context, data);
        }

        // Process errors and extensions from FlattenNode results
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
    #[instrument(skip(self, execution_context), name = "FlattenNode::execute")]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        // Execute the child node. `execution_context` can be borrowed mutably
        // because `collected_representations` borrows `data_for_flatten`, not `execution_context.data`.
        let normalized_path: Vec<&str> = self.path.iter().map(String::as_str).collect();
        let mut representations = traverse_and_collect(data, normalized_path.as_slice());
        match self.node.as_ref() {
            PlanNode::Fetch(fetch_node) => {
                let ProjectRepresentationsResult {
                    representations: filtered_representations,
                    indexes: filtered_repr_indexes,
                } = fetch_node.project_representations(execution_context, &representations);

                let result = fetch_node
                    .execute_for_projected_representations(
                        execution_context,
                        filtered_representations,
                        filtered_repr_indexes,
                    )
                    .await;
                // Process the result
                process_representations_result(result, &mut representations, execution_context);
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

#[async_trait]
impl ExecutablePlanNode for ConditionNode {
    #[instrument(skip(self, execution_context), name = "ConditionNode::execute")]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        // Get the condition variable from the context
        let condition_value: bool = match execution_context.variable_values {
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
                return if_clause.execute(execution_context, data).await;
            }
        } else if let Some(else_clause) = &self.else_clause {
            return else_clause.execute(execution_context, data).await;
        }
    }
}

#[async_trait]
impl ExecutableQueryPlan for QueryPlan {
    #[instrument(skip(self, execution_context))]
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

pub type SubgraphExecutorMap<'a> =
    HashMap<String, Arc<Box<executors::common::SubgraphExecutorType<'a>>>>;

pub struct QueryPlanExecutionContext<'a> {
    // Using `Value` provides flexibility
    pub variable_values: &'a Option<HashMap<String, Value>>,
    pub schema_metadata: &'a SchemaMetadata,
    pub subgraph_executor_map: &'a SubgraphExecutorMap<'a>,
    pub errors: Vec<GraphQLError>,
    pub extensions: HashMap<String, Value>,
}

impl QueryPlanExecutionContext<'_> {
    #[instrument(skip(self, execution_request))]
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: ExecutionRequest,
    ) -> ExecutionResult {
        debug!(
            "ExecutionRequest; Subgraph: {} Query:{} Variables: {:?}",
            subgraph_name, execution_request.query, execution_request.variables
        );
        let executor = self
            .subgraph_executor_map
            .get(subgraph_name)
            .expect("Subgraph executor not found");
        executor.execute(execution_request).await
    }

    #[instrument(
        name = "Project requires selections",
        skip(self, requires_selections, entity)
        fields(
            requires_selections = ?requires_selections,
            entity = ?entity
        )
    )]
    fn project_requires(&self, requires_selections: &Vec<SelectionItem>, entity: &Value) -> Value {
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
                            let type_name = match entity_obj.get("__typename") {
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
                                }
                                // If the projected value is not an object, it will be ignored
                            }
                        }
                    }
                }
                if (result_map.is_empty())
                    || (result_map.len() == 1 && result_map.contains_key("__typename"))
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
    name = "Check if entity satisfies type condition",
    skip(possible_types)
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
#[instrument]
fn traverse_and_collect<'a>(
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

#[instrument(skip(selection_set, schema_metadata, variable_values))]
fn project_selection_set_with_map(
    obj: &mut Map<String, Value>,
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
) -> Option<Map<String, Value>> {
    let type_name = match obj.get("__typename") {
        Some(Value::String(type_name)) => type_name,
        _ => type_name,
    }
    .to_string();
    let mut new_obj = Map::new();
    for selection in &selection_set.items {
        match selection {
            SelectionItem::Field(field) => {
                // Get the type fields for the current type
                let field_map = schema_metadata.type_fields.get(&type_name);
                // Type is not found in the schema
                field_map?;
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
                if field.name == "__typename" {
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

#[instrument(skip(selection_set, schema_metadata, variable_values))]
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

#[instrument(skip(operation, schema_metadata, variable_values))]
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

pub async fn execute_query_plan<'a>(
    query_plan: &QueryPlan,
    subgraph_executor_map: &'a SubgraphExecutorMap<'a>,
    variable_values: &Option<HashMap<String, Value>>,
    schema_metadata: &SchemaMetadata,
    operation: &OperationDefinition,
    has_introspection: bool,
) -> ExecutionResult {
    debug!("executing the query plan: {:?}", query_plan);
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

    #[cfg(debug_assertions)] // Only log in debug builds
    {
        result_extensions.insert("queryPlan".to_string(), serde_json::json!(query_plan));
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
