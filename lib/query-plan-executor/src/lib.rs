use async_trait::async_trait;
use futures::future::join_all;
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
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    vec,
};
use tokio::sync::Mutex;
use tracing::{debug, instrument, warn}; // For reading file in main

use crate::schema_metadata::SchemaMetadata;
mod deep_merge;
pub mod introspection;
pub mod schema_metadata;
pub mod validation;
mod value_from_ast;
pub mod variables;

#[async_trait]
trait ExecutablePlanNode {
    async fn execute(
        &self,
        execution_context_arc: Arc<QueryPlanExecutionContext<'_>>,
        representations: Arc<Vec<&mut Value>>,
    );
}

#[async_trait]
trait ExecutableQueryPlan {
    async fn execute(&self, execution_context_arc: Arc<QueryPlanExecutionContext<'_>>);
}

#[async_trait]
impl ExecutablePlanNode for PlanNode {
    async fn execute(
        &self,
        execution_context_arc: Arc<QueryPlanExecutionContext<'_>>,
        representations: Arc<Vec<&mut Value>>,
    ) {
        match self {
            PlanNode::Fetch(node) => node.execute(execution_context_arc, representations).await,
            PlanNode::Sequence(node) => node.execute(execution_context_arc, representations).await,
            PlanNode::Parallel(node) => node.execute(execution_context_arc, representations).await,
            PlanNode::Flatten(node) => node.execute(execution_context_arc, representations).await,
            PlanNode::Condition(node) => node.execute(execution_context_arc, representations).await,
            PlanNode::Subscription(node) => {
                // Subscriptions typically use a different protocol.
                // Execute the primary node for now.
                warn!(
            "Executing SubscriptionNode's primary as a normal node. Real subscription handling requires a different mechanism."
        );
                node.primary
                    .execute(execution_context_arc, representations)
                    .await
            }
            PlanNode::Defer(_) => {
                // Defer/Deferred execution is complex.
                warn!("DeferNode execution is not fully implemented.");
            }
        }
    }
}

#[async_trait]
trait ExecutableFetchNode {
    async fn execute_for_root(&self, execution_context_arc: Arc<QueryPlanExecutionContext<'_>>);
    async fn execute_for_representations(
        &self,
        execution_context_arc: Arc<QueryPlanExecutionContext<'_>>,
        representations: Arc<Vec<&mut Value>>,
    );
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
    #[instrument(skip(self, execution_context_arc), name = "FetchNode::execute")]
    async fn execute(
        &self,
        execution_context_arc: Arc<QueryPlanExecutionContext<'_>>,
        representations: Arc<Vec<&mut Value>>,
    ) {
        match &self.requires {
            Some(_) => {
                self.execute_for_representations(execution_context_arc, representations)
                    .await
            }
            None => self.execute_for_root(execution_context_arc).await,
        }
    }
}

#[async_trait]
impl ExecutableFetchNode for FetchNode {
    async fn execute_for_root(&self, execution_context_arc: Arc<QueryPlanExecutionContext<'_>>) {
        let variables =
            self.prepare_variables_for_fetch_node(execution_context_arc.variable_values);

        let fetch_result = execution_context_arc
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
        match fetch_result.errors {
            Some(errors) if !errors.is_empty() => {
                execution_context_arc.add_errors(errors).await;
            }
            _ => {}
        }

        if let Some(mut data) = fetch_result.data {
            self.apply_output_rewrites(
                &execution_context_arc.schema_metadata.possible_types,
                &mut data,
            );
            execution_context_arc.merge_data(data).await;
        }

        match fetch_result.extensions {
            Some(extensions) if !extensions.is_empty() => {
                execution_context_arc.merge_extensions(extensions).await;
            }
            _ => {}
        }
    }

    async fn execute_for_representations(
        &self,
        execution_context_arc: Arc<QueryPlanExecutionContext<'_>>,
        mut representations: Arc<Vec<&mut Value>>,
    ) {
        let representations = Arc::get_mut(&mut representations).unwrap();
        let mut filtered_repr_indexes = Vec::new();
        // 1. Filter representations based on requires (if present)
        let mut filtered_representations: Vec<Value> = Vec::new();
        let requires_nodes = self.requires.as_ref().unwrap();
        for (index, entity) in representations.iter().enumerate() {
            let entity_projected =
                execution_context_arc.project_requires(&requires_nodes.items, entity);
            if !entity_projected.is_null() {
                filtered_representations.push(entity_projected);
                filtered_repr_indexes.push(index);
            }
        }
        // No representations to fetch, do not call the subgraph
        if filtered_representations.is_empty() {
            return;
        }

        if let Some(input_rewrites) = &self.input_rewrites {
            for representation in &mut filtered_representations {
                for input_rewrite in input_rewrites {
                    // Apply input rewrites to each representation
                    input_rewrite.apply(
                        &execution_context_arc.schema_metadata.possible_types,
                        representation,
                    );
                }
            }
        }

        // 2. Prepare variables for fetch
        let mut variables = self
            .prepare_variables_for_fetch_node(execution_context_arc.variable_values)
            .unwrap_or_default();

        variables.insert(
            "representations".to_string(),
            Value::Array(filtered_representations),
        );

        let fetch_result = execution_context_arc
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

        // 5. Process the response

        // Process data
        if let Some(mut data) = fetch_result.data {
            self.apply_output_rewrites(
                &execution_context_arc.schema_metadata.possible_types,
                &mut data,
            );
            // Attempt to extract the _entities array mutably or take ownership
            let entities_option = match &mut data {
                Value::Object(map) => map.remove("_entities"), // Take ownership
                _ => None,
            };

            // Process _entities array
            match entities_option {
                Some(Value::Array(entities)) => {
                    for (entity, representation_index) in
                        entities.into_iter().zip(filtered_repr_indexes.iter_mut())
                    {
                        let representation_obj = representations
                            .get_mut(*representation_index)
                            .unwrap()
                            .as_object_mut()
                            .unwrap();
                        if let Value::Object(entity_obj) = entity {
                            deep_merge::deep_merge_objects(representation_obj, entity_obj);
                        }
                    }
                }
                _ => {
                    // Called with reps, but no _entities array found. Merge entire response as fallback.
                    warn!(
            "Fetch called with representations, but no '_entities' array found in response. Merging entire response data."
        );
                }
            }
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
        variable_values.as_ref().map(|variable_values| {
            variable_values
                .iter()
                .filter_map(|(variable_name, value)| {
                    if self
                        .variable_usages
                        .as_ref()
                        .is_some_and(|variable_usages| variable_usages.contains(variable_name))
                    {
                        Some((variable_name.to_string(), value.clone()))
                    } else {
                        None
                    }
                })
                .collect()
        })
    }
}

trait ApplyFetchRewrite {
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, value: &mut Value);
    fn apply_path(
        &self,
        _possible_types: &HashMap<String, Vec<String>>,
        _value: &mut Value,
        _path: &[String],
    ) {
    }
}

impl ApplyFetchRewrite for FetchRewrite {
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, value: &mut Value) {
        match self {
            FetchRewrite::KeyRenamer(renamer) => renamer.apply(possible_types, value),
            FetchRewrite::ValueSetter(setter) => setter.apply(possible_types, value),
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
    #[instrument(skip(self, execution_context_arc), name = "SequenceNode::execute")]
    async fn execute(
        &self,
        execution_context_arc: Arc<QueryPlanExecutionContext<'_>>,
        representations: Arc<Vec<&mut Value>>,
    ) {
        for node in &self.nodes {
            node.execute(execution_context_arc.clone(), representations.clone()) // No representations passed to child nodes
                .await;
        }
    }
}

#[async_trait]
impl ExecutablePlanNode for ParallelNode {
    #[instrument(skip(self, execution_context_arc), name = "ParallelNode::execute")]
    async fn execute(
        &self,
        execution_context_arc: Arc<QueryPlanExecutionContext<'_>>,
        representations: Arc<Vec<&mut Value>>,
    ) {
        let mut jobs = Vec::new();
        for node in &self.nodes {
            let job = node.execute(execution_context_arc.clone(), representations.clone());
            jobs.push(job);
        }
        join_all(jobs).await;
    }
}

#[async_trait]
impl ExecutablePlanNode for FlattenNode {
    #[instrument(skip(self, execution_context_arc), name = "FlattenNode::execute")]
    async fn execute(
        &self,
        execution_context_arc: Arc<QueryPlanExecutionContext<'_>>,
        _representations: Arc<Vec<&mut Value>>,
    ) {
        let mut data_for_flatten = execution_context_arc.data_mutex.lock().await;
        // Use the recursive traversal function on the temporarily owned data
        let representations = traverse_and_collect(
            &mut data_for_flatten, // Operate on the separated data
            self.path
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .as_slice(),
        );

        // Execute the child node. `execution_context` can be borrowed mutably
        // because `collected_representations` borrows `data_for_flatten`, not `execution_context.data`.
        let execution_context_arc = execution_context_arc.clone();
        self.node
            .execute(
                execution_context_arc,
                Arc::new(representations), // Pass representations borrowing data_for_flatten
            )
            .await
    }
}

#[async_trait]
impl ExecutablePlanNode for ConditionNode {
    #[instrument(skip(self, execution_context_arc), name = "ConditionNode::execute")]
    async fn execute(
        &self,
        execution_context_arc: Arc<QueryPlanExecutionContext<'_>>,
        representations: Arc<Vec<&mut Value>>,
    ) {
        // Get the condition variable from the context
        let condition_value: bool = match execution_context_arc.variable_values {
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
                if_clause
                    .execute(execution_context_arc, representations)
                    .await
            }
        } else if let Some(else_clause) = &self.else_clause {
            else_clause
                .execute(execution_context_arc, representations)
                .await
        }
    }
}

#[async_trait]
impl ExecutableQueryPlan for QueryPlan {
    #[instrument(skip(self, execution_context_arc))]
    async fn execute(&self, execution_context_arc: Arc<QueryPlanExecutionContext<'_>>) {
        if let Some(root_node) = &self.node {
            root_node
                .execute(
                    execution_context_arc.clone(),
                    Arc::new(vec![]), // No representations passed to the root node
                )
                .await
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Debug)]
pub struct QueryPlanExecutionContext<'a> {
    // Using `Value` provides flexibility
    pub variable_values: &'a Option<HashMap<String, Value>>,
    schema_metadata: &'a SchemaMetadata,
    executor: HTTPSubgraphExecutor<'a>,
    // Using `Value` as the main data structure, usually will be a JSON object
    pub data_mutex: Mutex<Value>,
    pub errors_mutex: Mutex<Vec<GraphQLError>>,
    pub extensions_mutex: Mutex<HashMap<String, Value>>,
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
        self.executor
            .execute(subgraph_name, execution_request)
            .await
    }

    async fn merge_extensions(&self, new_extensions: HashMap<String, Value>) {
        let mut extensions_lock = self.extensions_mutex.lock().await;
        if extensions_lock.is_empty() {
            *extensions_lock = new_extensions;
            return;
        }
        extensions_lock.extend(new_extensions);
    }

    async fn add_errors(&self, errors: Vec<GraphQLError>) {
        let mut errors_lock = self.errors_mutex.lock().await;
        if (*errors_lock).is_empty() {
            *errors_lock = errors;
            return;
        }
        // Add new errors to the existing list
        errors_lock.extend(errors);
    }
    async fn merge_data(&self, new_data: Value) {
        let mut data_lock = self.data_mutex.lock().await;
        match (&mut *data_lock, new_data) {
            (Value::Object(data_map), Value::Object(new_map)) => {
                deep_merge::deep_merge_objects(data_map, new_map);
            }
            (Value::Null, Value::Object(new_map)) => {
                *data_lock = Value::Object(new_map);
            }
            _ => {}
        }
    }

    #[instrument(skip(self))]
    fn project_requires(&self, requires_selections: &Vec<SelectionItem>, entity: &Value) -> Value {
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
                                    result_map.extend(projected_map);
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
            Value::Number(num) => Value::Number(num.clone()),
            Value::String(string) => Value::String(string.to_string()),
        }
    }
}

#[instrument]
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
/// handling '@' for array iteration, and collects the final values.
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

#[derive(Debug)]
struct HTTPSubgraphExecutor<'a> {
    subgraph_endpoint_map: &'a HashMap<String, String>,
    http_client: &'a reqwest::Client,
}

impl HTTPSubgraphExecutor<'_> {
    async fn _execute(
        &self,
        subgraph_name: &str,
        execution_request: ExecutionRequest,
    ) -> Result<ExecutionResult, reqwest::Error> {
        match self.subgraph_endpoint_map.get(subgraph_name) {
            Some(subgraph_endpoint) => {
                self.http_client
                    .post(subgraph_endpoint)
                    .json(&execution_request)
                    .send()
                    .await?
                    .json::<ExecutionResult>()
                    .await
            }
            None => Ok(ExecutionResult::from_error_message(format!(
                "Subgraph {} not found in endpoint map",
                subgraph_name
            ))),
        }
    }

    #[instrument(skip(self, execution_request))]
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: ExecutionRequest,
    ) -> ExecutionResult {
        self._execute(subgraph_name, execution_request)
            .await
            .unwrap_or_else(|e| {
                ExecutionResult::from_error_message(format!(
                    "Error executing subgraph {}: {}",
                    subgraph_name, e
                ))
            })
    }
}

fn project_selection_set_with_map(
    obj: &mut Map<String, Value>,
    errors: &mut Vec<GraphQLError>,
    selection_set: &SelectionSet,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
) -> HashSet<String> {
    let type_name = match obj.get("__typename") {
        Some(Value::String(type_name)) => type_name,
        _ => type_name,
    }
    .to_string();
    let mut response_keys: HashSet<String> = HashSet::new();
    for selection in &selection_set.items {
        match selection {
            SelectionItem::Field(field) => {
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
                response_keys.insert(response_key.to_string());
                if field.name == "__typename" {
                    obj.insert(response_key, Value::String(type_name.to_string()));
                    continue;
                }
                // Get the type fields for the current type
                let field_map = schema_metadata.type_fields.get(&type_name);
                // Type is not found in the schema
                if field_map.is_none() {
                    continue;
                }
                let field_map = field_map.unwrap();
                let field_type = field_map.get(&field.name);
                if field.name == "__schema" && type_name == "Query" {
                    obj.insert(
                        "__schema".to_string(),
                        schema_metadata.introspection_schema_root_json.clone(),
                    );
                }
                let field_val = obj.get_mut(&response_key);
                match (field_type, field_val) {
                    (Some(field_type), Some(field_val)) => {
                        project_selection_set(
                            field_val,
                            errors,
                            &field.selections,
                            field_type,
                            schema_metadata,
                            variable_values,
                        );
                    }
                    (Some(_field_type), None) => {
                        // If the field is not found in the object, set it to Null
                        obj.insert(response_key, Value::Null);
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
                    let response_keys_from_fragment = project_selection_set_with_map(
                        obj,
                        errors,
                        &inline_fragment.selections,
                        &type_name,
                        schema_metadata,
                        variable_values,
                    );
                    response_keys.extend(response_keys_from_fragment);
                }
            }
        }
    }
    response_keys
}

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
            // If data is an object, project the selection set with the object
            let response_keys = project_selection_set_with_map(
                obj,
                errors,
                selection_set,
                type_name,
                schema_metadata,
                variable_values,
            );

            // Replace the original object with the filtered object
            *obj = response_keys
                .iter()
                .map(|response_key| {
                    (
                        response_key.to_string(),
                        obj.remove(response_key).unwrap_or(Value::Null),
                    )
                })
                .collect();
        }
        _ => {}
    }
}

fn project_data_by_operation(
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

pub async fn execute_query_plan(
    query_plan: &QueryPlan,
    subgraph_endpoint_map: &HashMap<String, String>,
    variable_values: &Option<HashMap<String, Value>>,
    schema_metadata: &SchemaMetadata,
    operation: &OperationDefinition,
    has_introspection: bool,
    http_client: &reqwest::Client,
) -> ExecutionResult {
    debug!("executing the query plan: {:?}", query_plan);
    let http_executor = HTTPSubgraphExecutor {
        subgraph_endpoint_map,
        http_client,
    };
    let result_data = Value::Null; // Initial data is Null
    let result_errors = vec![]; // Initial errors are empty
    let result_extensions = HashMap::new(); // Initial extensions are empty
    let execution_context = QueryPlanExecutionContext {
        variable_values,
        executor: http_executor,
        schema_metadata,
        data_mutex: Mutex::new(result_data),
        errors_mutex: Mutex::new(result_errors),
        extensions_mutex: Mutex::new(result_extensions),
    };
    let execution_context_arc = Arc::new(execution_context);
    query_plan.execute(execution_context_arc.clone()).await;
    let execution_context = Arc::into_inner(execution_context_arc).unwrap();
    let mut result_data = execution_context.data_mutex.into_inner();
    let mut result_errors = execution_context.errors_mutex.into_inner();
    let mut result_extensions = execution_context.extensions_mutex.into_inner();
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
