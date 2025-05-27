use async_trait::async_trait;
use futures::future::join_all;
use graphql_parser::query::{
    Definition, Directive, Document, FragmentDefinition, OperationDefinition, Selection,
    SelectionSet, TypeCondition,
};
use query_planner::{
    ast::selection_item::SelectionItem,
    planner::plan_nodes::{
        ConditionNode, FetchNode, FlattenNode, InputRewrite, KeyRenamer, OutputRewrite,
        ParallelNode, PlanNode, QueryPlan, SequenceNode, ValueSetter,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, vec};

#[async_trait]
trait ExecutablePlanNode {
    async fn execute(
        &self,
        execution_context: &QueryPlanExecutionContext,
        data: Value,
        representations: Vec<Value>,
    ) -> (Value, Vec<Value>, Vec<GraphQLError>);
}

#[async_trait]
impl ExecutablePlanNode for PlanNode {
    async fn execute(
        &self,
        execution_context: &QueryPlanExecutionContext,
        data: Value,
        representations: Vec<Value>,
    ) -> (Value, Vec<Value>, Vec<GraphQLError>) {
        match self {
            PlanNode::Fetch(node) => node.execute(execution_context, data, representations).await,
            PlanNode::Sequence(node) => {
                node.execute(execution_context, data, representations).await
            }
            PlanNode::Parallel(node) => {
                node.execute(execution_context, data, representations).await
            }
            PlanNode::Flatten(node) => node.execute(execution_context, data, representations).await,
            PlanNode::Condition(node) => {
                node.execute(execution_context, data, representations).await
            }
            PlanNode::Subscription(node) => {
                // Subscriptions typically use a different protocol.
                // Execute the primary node for now.
                println!(
            "Warning: Executing SubscriptionNode's primary as a normal node. Real subscription handling requires a different mechanism."
        );
                node.primary
                    .execute(execution_context, data, representations)
                    .await
            }
            PlanNode::Defer(_) => {
                // Defer/Deferred execution is complex.
                println!("Warning: DeferNode execution is not fully implemented.");
                (data, representations, Vec::new()) // Return empty for now
            }
        }
    }
}

trait ExecutableFetchNode {
    async fn execute_for_root(
        &self,
        execution_context: &QueryPlanExecutionContext,
    ) -> (Value, Vec<GraphQLError>);
    async fn execute_for_representations(
        &self,
        execution_context: &QueryPlanExecutionContext,
        representations: Vec<Value>,
    ) -> (Vec<Value>, Vec<GraphQLError>);
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
    async fn execute(
        &self,
        execution_context: &QueryPlanExecutionContext,
        data: Value,
        representations: Vec<Value>,
    ) -> (Value, Vec<Value>, Vec<GraphQLError>) {
        // 1. Check if the operation is a root operation
        if self.requires.is_none() {
            let (data, errors) = self.execute_for_root(execution_context).await;
            return (data, representations, errors);
        }
        // 2. Execute the fetch for representations
        let (representations, errors) = self
            .execute_for_representations(execution_context, representations)
            .await;
        (data, representations, errors)
    }
}

impl ExecutableFetchNode for FetchNode {
    async fn execute_for_root(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
    ) -> (Value, Vec<GraphQLError>) {
        let variables = self.prepare_variables_for_fetch_node(execution_context.variable_values);

        let fetch_result = execution_context
            .execute(
                &self.service_name,
                ExecutionRequest {
                    query: Some(self.operation.0.to_string()),
                    operation_name: self.operation_name.clone(),
                    variables,
                    extensions: None,
                },
            )
            .await;

        // 5. Process the response
        let errors: Vec<GraphQLError> = fetch_result.errors.unwrap_or_default();

        // Process data
        let data: Value = match fetch_result.data {
            Some(mut data) => {
                self.apply_output_rewrites(
                    &execution_context.schema_metadata.possible_types,
                    &mut data,
                );
                data
            }
            _ => Value::Null,
        };
        (data, errors)
    }

    async fn execute_for_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        representations: Vec<Value>,
    ) -> (Vec<Value>, Vec<GraphQLError>) {
        let mut filtered_repr_indexes;
        let representations_length = representations.len();
        let mut final_representations: Vec<Value> = vec![Value::Null; representations_length]; // Initialize with the same length as representations
                                                                                               // 1. Filter representations based on requires (if present)
        let mut filtered_representations: Vec<Value>;
        match &self.requires {
            Some(requires_nodes) => {
                filtered_repr_indexes = Vec::new();
                filtered_representations = Vec::new();
                for (index, entity) in representations.into_iter().enumerate() {
                    let entity_projected =
                        execution_context.project_requires(&requires_nodes.items, &entity);
                    if !entity_projected.is_null() {
                        filtered_representations.push(entity_projected);
                        filtered_repr_indexes.push(index);
                    }
                }
            }
            _ => {
                // No requires, use all representations.
                filtered_representations = representations; // Use the owned Vec directly
                let representation_length = filtered_representations.len();
                if representation_length > 0 {
                    filtered_repr_indexes = (0..(representation_length - 1)).collect();
                } else {
                    filtered_repr_indexes = Vec::new(); // No indexes to filter
                }
            }
        };
        // No representations to fetch, do not call the subgraph
        if filtered_representations.is_empty() {
            return (final_representations, Vec::new());
        }
        let processed_representations: Vec<Value> = filtered_representations
            .into_iter()
            .map(|repr| match &self.input_rewrites {
                Some(input_rewrites) => input_rewrites.iter().fold(repr, |repr, input_rewrite| {
                    input_rewrite.apply(&execution_context.schema_metadata.possible_types, repr)
                }),
                None => repr,
            })
            .collect();

        // 2. Prepare variables for fetch
        let mut variables = self
            .prepare_variables_for_fetch_node(execution_context.variable_values)
            .unwrap_or_default();

        variables.insert(
            "representations".to_string(),
            Value::Array(processed_representations),
        );

        let fetch_result = execution_context
            .execute(
                &self.service_name,
                ExecutionRequest {
                    query: Some(self.operation.0.to_string()),
                    operation_name: self.operation_name.clone(),
                    variables: Some(variables),
                    extensions: None,
                },
            )
            .await;

        // 5. Process the response
        let errors: Vec<GraphQLError> = fetch_result.errors.unwrap_or_default();

        // Process data
        if let Some(mut data) = fetch_result.data {
            self.apply_output_rewrites(
                &execution_context.schema_metadata.possible_types,
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
                    for (i, entity) in entities.into_iter().enumerate() {
                        let representation_index = filtered_repr_indexes.get(i);
                        match representation_index {
                            Some(&representation_index) => {
                                final_representations[representation_index] = entity;
                            }
                            None => {
                                println!(
                        "Warning: Entity index {} out of bounds for representations. Skipping merge.",
                        i
                    );
                            }
                        }
                    }
                }
                _ => {
                    // Called with reps, but no _entities array found. Merge entire response as fallback.
                    println!(
            "Warning: Fetch called with representations, but no '_entities' array found in response. Merging entire response data."
        );
                }
            }
        }

        (final_representations, errors)
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
        if self.variable_usages.is_empty() {
            None
        } else {
            variable_values.as_ref().map(|variable_values| {
                variable_values
                    .iter()
                    .filter_map(|(variable_name, value)| {
                        if self.variable_usages.contains(variable_name) {
                            Some((variable_name.to_string(), value.clone()))
                        } else {
                            None
                        }
                    })
                    .collect()
            })
        }
    }
}

trait ApplyOutputRewrite {
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, value: &mut Value);
    fn apply_path(
        &self,
        _possible_types: &HashMap<String, Vec<String>>,
        _value: &mut Value,
        _path: &[String],
    ) {
    }
}

impl ApplyOutputRewrite for OutputRewrite {
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, value: &mut Value) {
        match self {
            OutputRewrite::KeyRenamer(renamer) => renamer.apply(possible_types, value),
        }
    }
}

impl ApplyOutputRewrite for KeyRenamer {
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
                for item in arr.iter_mut() {
                    self.apply_path(possible_types, item, path);
                }
            }
            Value::Object(obj) => {
                let type_condition = current_segment.strip_prefix("... on ");
                match type_condition {
                    Some(type_condition) => {
                        if entity_satisfies_type_condition(possible_types, obj, type_condition) {
                            self.apply_path(possible_types, value, remaining_path)
                        }
                    }
                    _ => {
                        if remaining_path.is_empty() {
                            if let Some(val) = obj.get(current_segment) {
                                // Rename the key
                                obj.insert(self.rename_key_to.to_string(), val.clone());
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

trait ApplyInputRewrite {
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, data: Value) -> Value;
    fn apply_path(
        &self,
        _possible_types: &HashMap<String, Vec<String>>,
        _data: Value,
        _path: &[String],
    ) -> Value {
        Value::Null
    }
}

impl ApplyInputRewrite for InputRewrite {
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, data: Value) -> Value {
        match self {
            InputRewrite::ValueSetter(setter) => setter.apply(possible_types, data),
        }
    }
}
impl ApplyInputRewrite for ValueSetter {
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, data: Value) -> Value {
        self.apply_path(possible_types, data, &self.path)
    }
    // Applies value setting on a Value (returns a new Value)
    fn apply_path(
        &self,
        possible_types: &HashMap<String, Vec<String>>,
        data: Value,
        path: &[String],
    ) -> Value {
        if path.is_empty() {
            return self.set_value_to.clone();
        }

        match data {
            Value::Array(arr) => {
                let new_arr = arr
                    .into_iter()
                    .map(|item| self.apply_path(possible_types, item, path))
                    .collect();
                Value::Array(new_arr)
            }
            Value::Object(mut map) => {
                let current_key = &path[0];
                let remaining_path = &path[1..];

                if let Some(type_condition) = current_key.strip_prefix("... on ") {
                    if entity_satisfies_type_condition(possible_types, &map, type_condition) {
                        let data = Value::Object(map);
                        return self.apply_path(possible_types, data, remaining_path);
                    }
                }

                if remaining_path.is_empty() {
                    map.insert(current_key.to_string(), self.set_value_to.clone());
                } else {
                    let entry_value = map.entry(current_key.to_string()).or_insert(Value::Null);
                    let current_val = entry_value.clone();
                    let new_val = self.apply_path(possible_types, current_val, remaining_path);
                    *entry_value = new_val;
                }
                Value::Object(map)
            }
            _ => {
                println!(
                    "Warning: Trying to apply ValueSetter path {:?} to non-object/array type: {:?}",
                    path, data
                );
                data
            }
        }
    }
}

#[async_trait]
impl ExecutablePlanNode for SequenceNode {
    async fn execute(
        &self,
        execution_context: &QueryPlanExecutionContext,
        mut data: Value,
        mut representations: Vec<Value>,
    ) -> (Value, Vec<Value>, Vec<GraphQLError>) {
        let mut errors: Vec<GraphQLError> = Vec::new();
        let is_data_merge = representations.is_empty();
        for node in &self.nodes {
            // Avoid extra cloning of data if not needed
            let data_clone = if is_data_merge {
                data.clone()
            } else {
                Value::Null // Placeholder for data
            };
            // Exit with ? if an inner execution fails
            let (result_data, result_representations, result_errors) = node
                .execute(execution_context, data_clone, representations.clone()) // No representations passed to child nodes
                .await;
            if is_data_merge {
                deep_merge(&mut data, result_data);
            } else {
                for (i, result_representation) in result_representations.into_iter().enumerate() {
                    let current_representation = representations.get_mut(i);
                    if let Some(current_repr) = current_representation {
                        // Merge the result into the current representation
                        deep_merge(current_repr, result_representation);
                    } else {
                        println!(
                            "Warning: Entity index {} out of bounds for representations. Skipping merge.",
                            i,
                        );
                    }
                }
            }

            errors.extend(result_errors);
        }
        (data, representations, errors)
    }
}

#[async_trait]
impl ExecutablePlanNode for ParallelNode {
    async fn execute(
        &self,
        execution_context: &QueryPlanExecutionContext,
        mut data: Value,
        mut representations: Vec<Value>,
    ) -> (Value, Vec<Value>, Vec<GraphQLError>) {
        let mut jobs = Vec::new();
        let mut errors: Vec<GraphQLError> = Vec::new();
        let is_data_merge = representations.is_empty();
        for node in &self.nodes {
            let data_clone = if is_data_merge {
                data.clone()
            } else {
                Value::Null // Placeholder for data
            };
            let job = node.execute(execution_context, data_clone, representations.clone());
            jobs.push(job);
        }
        let results = join_all(jobs).await;
        for (result_data, result_representations, result_errors) in results {
            if is_data_merge {
                deep_merge(&mut data, result_data);
            } else {
                for (i, result) in result_representations.into_iter().enumerate() {
                    let current_representation = representations.get_mut(i);
                    if let Some(current_repr) = current_representation {
                        // Merge the result into the current representation
                        deep_merge(current_repr, result);
                    } else {
                        println!(
                            "Warning: Entity index {} out of bounds for representations. Skipping merge.",
                            i,
                        );
                    }
                }
            }
            errors.extend(result_errors);
        }
        // Return the merged representations
        (data, representations, errors)
    }
}

#[async_trait]
impl ExecutablePlanNode for FlattenNode {
    async fn execute(
        &self,
        execution_context: &QueryPlanExecutionContext,
        mut data: Value,
        representations: Vec<Value>,
    ) -> (Value, Vec<Value>, Vec<GraphQLError>) {
        let errors: Vec<GraphQLError>;

        // Use the recursive traversal function on the temporarily owned data
        let mut collected_representations = traverse_and_collect(
            &mut data, // Operate on the separated data
            self.path
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .as_slice(),
        );

        if !collected_representations.is_empty() {
            let representations: Vec<Value> = collected_representations
                .iter()
                .map(|v| (**v).clone())
                .collect();
            // Execute the child node. `execution_context` can be borrowed mutably
            // because `collected_representations` borrows `data_for_flatten`, not `execution_context.data`.
            let (_result_data, result_representations, result_errors) = self
                .node
                .execute(
                    execution_context,
                    Value::Null,     // Pass Null as data to the child node
                    representations, // Pass representations borrowing data_for_flatten
                )
                .await;
            errors = result_errors;
            // Merge the results back into the data
            for (i, result_representation) in result_representations.into_iter().enumerate() {
                match collected_representations.get_mut(i) {
                    Some(current_repr) => {
                        // Merge the entity into the current representation
                        deep_merge(current_repr, result_representation);
                    }
                    None => {
                        println!(
                            "Warning: Entity index {} out of bounds for collected representations. Skipping merge.",
                            i,
                        );
                    }
                }
            }
            // Borrows held by collected_representations end here
        } else {
            // Log if no representations were found for the path.
            println!(
                "Info: Flatten node produced no representations for path {:?}. Skipping child node execution.",
                self.path
            );
            errors = Vec::new();
        };

        (data, representations, errors)
    }
}

#[async_trait]
impl ExecutablePlanNode for ConditionNode {
    async fn execute(
        &self,
        execution_context: &QueryPlanExecutionContext,
        data: Value,
        representations: Vec<Value>,
    ) -> (Value, Vec<Value>, Vec<GraphQLError>) {
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
            match &self.if_clause {
                Some(if_clause) => {
                    if_clause
                        .execute(execution_context, data, representations)
                        .await
                }
                None => {
                    // If no if clause, return the original representations
                    let errors = Vec::new();
                    (data, representations, errors)
                }
            }
        } else {
            match &self.else_clause {
                Some(else_clause) => {
                    else_clause
                        .execute(execution_context, data, representations)
                        .await
                }
                None => {
                    // If no else clause, do nothing
                    let errors = Vec::new();
                    (data, representations, errors)
                }
            }
        }
    }
}

trait ExecutableQueryPlan {
    async fn execute(&self, execution_context: QueryPlanExecutionContext) -> ExecutionResult;
}

impl ExecutableQueryPlan for QueryPlan {
    async fn execute(&self, execution_context: QueryPlanExecutionContext<'_>) -> ExecutionResult {
        let data = Value::Null; // Placeholder for data
        let representations = Vec::new(); // Placeholder for representations
                                          // Execute the root node
                                          // The ? operator will propagate errors upwards
        match &self.node {
            Some(root_node) => {
                let (data, _representations, errors) = root_node
                    .execute(
                        &execution_context,
                        data,
                        representations, // No initial representations
                    )
                    .await;
                ExecutionResult {
                    data: Some(data),
                    errors: if errors.is_empty() {
                        None
                    } else {
                        Some(errors)
                    },
                    extensions: None,
                }
            }
            None => {
                // Handle case where QueryPlan has no node (though checked earlier)
                ExecutionResult {
                    data: None,
                    errors: Some(vec![GraphQLError {
                        message: "QueryPlan has no node".to_string(),
                        location: None,
                        path: None,
                        extensions: None,
                    }]),
                    extensions: None,
                }
            }
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
                location: None,
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
    pub location: Option<Vec<GraphQLErrorLocation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<Value>>, // Path can be string or number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, Value>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GraphQLErrorLocation {
    pub line: u32,
    pub column: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
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
}

impl QueryPlanExecutionContext<'_> {
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: ExecutionRequest,
    ) -> ExecutionResult {
        self.executor
            .execute(subgraph_name, execution_request)
            .await
    }

    fn project_requires(&self, requires_selections: &Vec<SelectionItem>, entity: &Value) -> Value {
        if requires_selections.is_empty() {
            return entity.clone();
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
                let mut result = Value::Object(serde_json::Map::new());
                for requires_selection in requires_selections {
                    match &requires_selection {
                        SelectionItem::Field(requires_selection) => {
                            let field_name = requires_selection.name.to_string();
                            let original = entity_obj.get(&field_name).unwrap_or(&Value::Null);
                            let projected_value: Value = self
                                .project_requires(&requires_selection.selections.items, original);
                            if !projected_value.is_null() {
                                let field_name = requires_selection.name.to_string();
                                let result_map = result.as_object_mut().unwrap();
                                result_map.insert(field_name, projected_value);
                            }
                        }
                        SelectionItem::InlineFragment(requires_selection) => {
                            if entity_satisfies_type_condition(
                                &self.schema_metadata.possible_types,
                                entity_obj,
                                &requires_selection.type_condition,
                            ) {
                                let projected = self
                                    .project_requires(&requires_selection.selections.items, entity);
                                deep_merge(&mut result, projected);
                            }
                        }
                    }
                }
                let result_map = result.as_object_mut().unwrap();
                if (result_map.is_empty())
                    || (result_map.len() == 1 && result_map.contains_key("__typename"))
                {
                    Value::Null
                } else {
                    result
                }
            }
            _ => entity.clone(),
        }
    }
}

fn entity_satisfies_type_condition(
    possible_types: &HashMap<String, Vec<String>>,
    entity_map: &serde_json::Map<String, Value>,
    type_condition: &str,
) -> bool {
    match entity_map.get("__typename") {
        Some(Value::String(entity_type_name)) => {
            if entity_type_name == type_condition {
                true
            } else {
                let possible_types_for_type_condition = possible_types.get(type_condition);
                match possible_types_for_type_condition {
                    Some(possible_types_for_type_condition) => {
                        possible_types_for_type_condition.contains(&entity_type_name.to_string())
                    }
                    None => {
                        // If no possible types are found, return false
                        false
                    }
                }
            }
        }
        _ => false,
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
        (current_data, []) => vec![current_data], // Base case: No more path segments,
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

// Deeply merges two serde_json::Values (mutates target in place)
fn deep_merge(target: &mut Value, source: Value) {
    match (target, source) {
        // 1. Source is Null: Do nothing
        (_, Value::Null) => {} // Keep target as is

        // 2. Both are Objects: Merge recursively
        (Value::Object(target_map), Value::Object(source_map)) => {
            for (key, source_val) in source_map {
                // Optimization: If source_val is Null, we could skip, but deep_merge handles it.
                let target_entry = target_map.entry(key).or_insert(Value::Null);
                deep_merge(target_entry, source_val);
            }
        }

        // 3. Both are Arrays of same length: Merge elements
        (Value::Array(target_arr), Value::Array(source_arr))
            if target_arr.len() == source_arr.len() =>
        {
            for (t, s) in target_arr.iter_mut().zip(source_arr.into_iter()) {
                // Recurse for elements. If s is Null, the recursive call handles it.
                deep_merge(t, s);
            }
        }

        // 4. Fallback: Source is not Null, and cases 2 & 3 didn't match. Replace target with source.
        (target_val, source) => {
            // source is guaranteed not Null here due to arm 1
            *target_val = source;
        }
    }
}

// --- Main Function (for testing) ---

#[derive(Debug)]
struct HTTPSubgraphExecutor<'a> {
    subgraph_endpoint_map: &'a HashMap<String, String>,
    http_client: reqwest::Client,
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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SchemaMetadata {
    pub possible_types: HashMap<String, Vec<String>>,
    pub enum_values: HashMap<String, Vec<String>>,
    pub type_fields: HashMap<String, HashMap<String, String>>,
}

fn should_skip_per_variables(
    directives: &Vec<Directive<'static, String>>,
    variable_values: &Option<HashMap<String, Value>>,
) -> bool {
    for directive in directives {
        if directive.name == "skip" {
            for (arg_name, arg_value) in &directive.arguments {
                if *arg_name == "if" {
                    let mut if_value = None;
                    if let Some(variable_values) = variable_values {
                        let arg_value = arg_value.to_string();
                        let arg_value_without_dollar = arg_value.strip_prefix("$");
                        match arg_value_without_dollar {
                            Some(arg_value) => {
                                // Check if the variable exists in the variable values
                                // and get its value
                                // Note: The original code used `arg_value.to_string()`, which is incorrect
                                // because it includes the '$' sign. We need to strip it first.
                                if let Some(value) = variable_values.get(arg_value) {
                                    if_value = Some(value);
                                }
                            }
                            None => {
                                println!(
                                    "Warning: Skip directive found with invalid variable name: {}",
                                    arg_value
                                );
                            }
                        }
                    }
                    match if_value {
                        Some(Value::Bool(if_value)) => {
                            return *if_value; // Skip if the condition is true
                        }
                        Some(_) => {
                            println!(
                                "Warning: Skip directive found with non-boolean if argument: {}",
                                arg_value
                            );
                            return true; // Skip if not boolean
                        }
                        None => {
                            println!(
                                "Warning: Skip directive found with unknown variable: {}",
                                arg_value
                            );
                            return false;
                        }
                    }
                }
            }
        } else if directive.name == "include" {
            for (arg_name, arg_value) in &directive.arguments {
                if *arg_name == "if" {
                    let mut if_value = None;
                    if let Some(variable_values) = variable_values {
                        let arg_value = arg_value.to_string();
                        let arg_value_without_dollar = arg_value.strip_prefix("$");
                        match arg_value_without_dollar {
                            Some(arg_value) => {
                                // Check if the variable exists in the variable values
                                // and get its value
                                // Note: The original code used `arg_value.to_string()`, which is incorrect
                                // because it includes the '$' sign. We need to strip it first.
                                if let Some(value) = variable_values.get(arg_value) {
                                    if_value = Some(value);
                                }
                            }
                            None => {
                                println!(
                                    "Warning: Skip directive found with invalid variable name: {}",
                                    arg_value
                                );
                            }
                        }
                    }
                    match if_value {
                        Some(Value::Bool(if_value)) => {
                            return !*if_value; // Skip if the condition is false
                        }
                        Some(_) => {
                            println!(
                                "Warning: Include directive found with non-boolean if argument: {}",
                                arg_value
                            );
                            return false; // Skip if not boolean
                        }
                        None => {
                            println!(
                                "Warning: Include directive found with unknown variable: {} on {:#?}",
                                arg_value, variable_values
                            );
                            return true; // Skip if not found
                        }
                    }
                }
            }
        }
    }
    false // Default to false if no skip directive found
}

fn project_selection_set(
    data: &Value,
    selection_set: &SelectionSet<'static, String>,
    type_name: &str,
    schema_metadata: &SchemaMetadata,
    fragments: &HashMap<String, &FragmentDefinition<'static, String>>,
    variable_values: &Option<HashMap<String, Value>>,
) -> Value {
    // If selection_set is empty, return the original data
    let type_name = match data.get("__typename") {
        Some(Value::String(type_name)) => type_name,
        _ => type_name,
    };
    // Get the type fields for the current type
    let field_map = schema_metadata.type_fields.get(type_name);
    match (
        selection_set,
        // Get enum values for the current type
        schema_metadata.enum_values.get(type_name),
        data,
    ) {
        // In case of composite type
        (selection_set, _, Value::Object(obj)) => {
            // Type is not found in the schema
            if field_map.is_none() {
                if selection_set.items.is_empty() {
                    return data.clone(); // No fields to project, return original data
                }
                return Value::Null; // No fields found for the type
            }
            let field_map = field_map.unwrap();
            let mut result = Value::Object(serde_json::Map::new());
            for selection in &selection_set.items {
                match selection {
                    Selection::Field(field) => {
                        if should_skip_per_variables(&field.directives, variable_values) {
                            continue;
                        }
                        let response_key = field.alias.as_ref().unwrap_or(&field.name).to_string();
                        let result_map = result.as_object_mut().unwrap();
                        if field.name == "__typename" {
                            result_map.insert(response_key, Value::String(type_name.to_string()));
                            continue;
                        }
                        let field_type = field_map.get(&field.name);
                        let field_val = obj.get(&response_key);
                        match (field_type, field_val) {
                            (Some(field_type), Some(field_val)) => {
                                let projected = project_selection_set(
                                    field_val,
                                    &field.selection_set,
                                    field_type,
                                    schema_metadata,
                                    fragments,
                                    variable_values,
                                );
                                result_map.insert(response_key, projected);
                            }
                            (Some(_field_type), None) => {
                                // If the field is not found in the object, set it to Null
                                result_map.insert(response_key, Value::Null);
                            }
                            (None, _) => {}
                        }
                    }
                    Selection::InlineFragment(inline_fragment) => {
                        if should_skip_per_variables(&inline_fragment.directives, variable_values) {
                            continue;
                        }
                        match &inline_fragment.type_condition {
                            Some(TypeCondition::On(type_condition)) => {
                                if entity_satisfies_type_condition(
                                    &schema_metadata.possible_types,
                                    obj,
                                    type_condition,
                                ) {
                                    let type_name = obj.get("__typename");
                                    match type_name {
                                        Some(Value::String(type_name)) => {
                                            let projected = project_selection_set(
                                                data,
                                                &inline_fragment.selection_set,
                                                type_name,
                                                schema_metadata,
                                                fragments,
                                                variable_values,
                                            );
                                            deep_merge(&mut result, projected);
                                        }
                                        _ => {
                                            // Handle case where type_name is not a string
                                            println!(
                                                "Warning: Type name is not a string: {:?}",
                                                type_name
                                            );
                                        }
                                    }
                                }
                            }
                            None => {}
                        }
                    }
                    Selection::FragmentSpread(fragment_spread) => {
                        if should_skip_per_variables(&fragment_spread.directives, variable_values) {
                            continue;
                        }
                        match fragments.get(&fragment_spread.fragment_name) {
                            Some(fragment) => {
                                match &fragment.type_condition {
                                    TypeCondition::On(type_condition) => {
                                        if entity_satisfies_type_condition(
                                            &schema_metadata.possible_types,
                                            obj,
                                            type_condition,
                                        ) {
                                            let type_name = obj.get("__typename");
                                            match type_name {
                                                Some(Value::String(type_name)) => {
                                                    let projected = project_selection_set(
                                                        data,
                                                        &fragment.selection_set,
                                                        type_name,
                                                        schema_metadata,
                                                        fragments,
                                                        variable_values,
                                                    );
                                                    deep_merge(&mut result, projected);
                                                }
                                                _ => {
                                                    // Handle case where type_name is not a string
                                                    println!(
                                                        "Warning: Type name is not a string: {:?}",
                                                        type_name
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Handle case where fragment is not found
                                println!(
                                    "Warning: Fragment {} not found in fragments map",
                                    fragment_spread.fragment_name
                                );
                            }
                        }
                    }
                }
            }
            result
        }
        // In case of an array
        (_, _, Value::Array(arr)) => Value::Array(
            arr.iter()
                .map(|item| {
                    project_selection_set(
                        item,
                        selection_set,
                        type_name,
                        schema_metadata,
                        fragments,
                        variable_values,
                    )
                })
                .collect(),
        ),
        // In case of enum type with a string
        (_, Some(enum_values), Value::String(value)) => {
            // Check if the value is in the enum values
            if enum_values.contains(&value.to_string()) {
                Value::String(value.to_string())
            } else {
                Value::Null // If not found, return Null
            }
        }
        (_, _, value) => value.clone(), // No fields found for the type
    }
}

fn project_data_by_operation(
    data: Value,
    document: &graphql_parser::query::Document<'static, String>,
    schema_metadata: &SchemaMetadata,
    variable_values: &Option<HashMap<String, Value>>,
) -> Value {
    let mut root_type_name = "Query"; // Default to Query
    let mut selection_set: Option<&SelectionSet<'static, String>> = None;
    let mut fragments: HashMap<String, &FragmentDefinition<'static, String>> = HashMap::new();
    for def in &document.definitions {
        match def {
            Definition::Operation(OperationDefinition::Query(query)) => {
                root_type_name = "Query";
                selection_set = Some(&query.selection_set);
            }
            Definition::Operation(OperationDefinition::Mutation(mutation)) => {
                root_type_name = "Mutation";
                selection_set = Some(&mutation.selection_set);
            }
            Definition::Operation(OperationDefinition::Subscription(subscription)) => {
                root_type_name = "Subscription";
                selection_set = Some(&subscription.selection_set);
            }
            Definition::Operation(OperationDefinition::SelectionSet(query_selection_set)) => {
                root_type_name = "Query";
                selection_set = Some(query_selection_set);
            }
            Definition::Fragment(fragment_def) => {
                fragments.insert(fragment_def.name.to_string(), fragment_def);
            }
        }
    }
    // Project the data based on the selection set
    project_selection_set(
        &data,
        selection_set.unwrap(),
        root_type_name,
        schema_metadata,
        &fragments,
        variable_values,
    )
}

pub async fn execute_query_plan(
    query_plan: &QueryPlan,
    subgraph_endpoint_map: &HashMap<String, String>,
    variable_values: &Option<HashMap<String, Value>>,
    schema_metadata: &SchemaMetadata,
    document: &Document<'static, String>,
) -> ExecutionResult {
    let http_executor = HTTPSubgraphExecutor {
        subgraph_endpoint_map,
        http_client: reqwest::Client::new(),
    };
    let execution_context = QueryPlanExecutionContext {
        variable_values,
        executor: http_executor,
        schema_metadata,
    };
    let mut result = query_plan.execute(execution_context).await;
    if result.data.is_some() {
        result.data = Some(project_data_by_operation(
            result.data.unwrap(),
            document,
            schema_metadata,
            variable_values,
        ));
    }
    result
}
