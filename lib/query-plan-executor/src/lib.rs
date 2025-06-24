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
use std::collections::HashMap;
use tracing::{instrument, warn};

use crate::{
    deep_merge::{deep_merge, deep_merge_objects},
    executors::map::SubgraphExecutorMap,
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
                warn!("Executing SubscriptionNode's primary as a normal node. Real subscription handling requires a different mechanism.");
                node.primary.execute(execution_context, data).await
            }
            PlanNode::Defer(_) => {
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
        execution_context.errors.extend(errors);
    }
    if let Some(extensions) = extensions {
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
    representations: &mut Vec<&mut Value>,
    execution_context: &mut QueryPlanExecutionContext<'_>,
) {
    if let Some(entities) = result.entities {
        for (entity, index) in entities.into_iter().zip(result.indexes.into_iter()) {
            if let Some(representation) = representations.get_mut(index) {
                deep_merge(representation, entity);
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
    ) -> ExecutionResult {
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
        filtered_repr_indexes: Vec<usize>,
    ) -> ExecuteForRepresentationsResult {
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

        let fetch_result = execution_context
            .subgraph_executor_map
            .execute(&self.service_name, execution_request)
            .await;

        let entities = if let Some(mut data) = fetch_result.data {
            self.apply_output_rewrites(
                &execution_context.schema_metadata.possible_types,
                &mut data,
            );
            match data {
                Value::Object(mut obj) => match obj.remove("_entities") {
                    Some(Value::Array(arr)) => Some(arr),
                    _ => None,
                },
                _ => None,
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
                    None
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
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        for node in &self.nodes {
            node.execute(execution_context, data).await;
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
    if let Some(new_data) = fetch_result.data {
        if data.is_null() {
            *data = new_data;
        } else {
            deep_merge(data, new_data);
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
    #[instrument(level = "trace", skip_all, name = "ParallelNode::execute", fields(
          nodes_count = %self.nodes.len()
      ))]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        let mut fetch_jobs = vec![];
        let mut flatten_jobs = vec![];
        let mut flatten_paths = vec![];

        for node in &self.nodes {
            match node {
                PlanNode::Fetch(fetch_node) => {
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
                            continue;
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

        let flatten_results = futures::future::join_all(flatten_jobs).await;
        let fetch_results = futures::future::join_all(fetch_jobs).await;

        for (result, path) in flatten_results.into_iter().zip(flatten_paths) {
            if let Some(entities) = result.entities {
                let mut collected_representations = traverse_and_collect(data, &path);
                for (entity, index) in entities.into_iter().zip(result.indexes.into_iter()) {
                    if let Some(representation) = collected_representations.get_mut(index) {
                        deep_merge(representation, entity);
                    }
                }
            }
            process_errors_and_extensions(execution_context, result.errors, result.extensions);
        }

        for fetch_result in fetch_results {
            process_root_result(fetch_result, execution_context, data);
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
    #[instrument(level = "trace", skip_all, name = "ConditionNode::execute")]
    async fn execute(
        &self,
        execution_context: &mut QueryPlanExecutionContext<'_>,
        data: &mut Value,
    ) {
        let condition_value: bool = match execution_context.variable_values {
            Some(ref variable_values) => match variable_values.get(&self.condition) {
                // Check if the value is a boolean
                Some(value) => match value {
                    Value::Bool(b) => *b,
                    _ => true, // Default to true if not a boolean
                },
                // If the variable is not found, default to false
                None => false,
            },
            // No variable values provided, default to false
            None => false,
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
    pub path: Option<Vec<Value>>,
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
    pub variable_values: &'a Option<HashMap<String, Value>>,
    pub schema_metadata: &'a SchemaMetadata,
    pub subgraph_executor_map: &'a SubgraphExecutorMap,
    pub errors: Vec<GraphQLError>,
    pub extensions: HashMap<String, Value>,
}

impl<'a> QueryPlanExecutionContext<'a> {
    #[instrument(
          level = "trace",
          skip_all,
          fields(
              requires_selections = ?requires_selections.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
              entity = ?entity
          )
      )]
    pub fn project_requires(&self, requires_selections: &[SelectionItem], entity: &Value) -> Value {
        if requires_selections.is_empty() {
            return entity.clone();
        }

        let mut result_map = Map::new();
        self.project_requires_recursive(requires_selections, entity, &mut result_map);

        if result_map.is_empty()
            || (result_map.len() == 1 && result_map.contains_key(TYPENAME_FIELD))
        {
            Value::Null
        } else {
            Value::Object(result_map)
        }
    }

    fn project_requires_recursive(
        &self,
        requires_selections: &[SelectionItem],
        entity: &Value,
        result_map: &mut Map<String, Value>,
    ) {
        if requires_selections.is_empty() {
            if let Value::Object(entity_map) = entity {
                deep_merge_objects(result_map, entity_map.clone());
            }
            return;
        }

        let entity_obj = match entity {
            Value::Object(obj) => obj,
            _ => return,
        };

        for requires_selection in requires_selections {
            match requires_selection {
                SelectionItem::Field(requires_selection) => {
                    let field_name = &requires_selection.name;
                    let response_key = requires_selection.selection_identifier();
                    let original = entity_obj
                        .get(field_name)
                        .unwrap_or(entity_obj.get(response_key).unwrap_or(&Value::Null));
                    let projected_value =
                        self.project_requires(&requires_selection.selections.items, original);
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
                        self.project_requires_recursive(
                            &requires_selection.selections.items,
                            entity,
                            result_map,
                        );
                    }
                }
            }
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
            None => false,
        }
    }
}

#[instrument(level = "trace", skip_all, fields(
    current_type = ?current_data,
    remaining_path = ?remaining_path
))]
pub fn traverse_and_collect<'a>(
    current_data: &'a mut Value,
    remaining_path: &[&str],
) -> Vec<&'a mut Value> {
    let mut current_values = vec![current_data];
    let mut next_values = Vec::new();

    for segment in remaining_path {
        if current_values.is_empty() {
            return vec![];
        }

        for value in current_values.drain(..) {
            match (value, *segment) {
                (Value::Object(obj), key) => {
                    if let Some(next_value) = obj.get_mut(key) {
                        next_values.push(next_value);
                    }
                }
                (Value::Array(arr), "@") => {
                    next_values.extend(arr.iter_mut());
                }
                _ => {}
            }
        }
        std::mem::swap(&mut current_values, &mut next_values);
    }

    if remaining_path.is_empty() {
        if let Some(val) = current_values.pop() {
            if let Value::Array(arr) = val {
                return arr.iter_mut().collect();
            } else {
                return vec![val];
            }
        } else {
            return vec![];
        }
    }

    current_values
}

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
) {
    let type_name = match obj.get(TYPENAME_FIELD) {
        Some(Value::String(type_name)) => type_name.clone(),
        _ => type_name.to_string(),
    };
    let field_map = schema_metadata.type_fields.get(&type_name);

    let selected_fields: HashMap<String, &SelectionItem> = selection_set
        .items
        .iter()
        .filter_map(|item| {
            if let SelectionItem::Field(field) = item {
                if let Some(ref skip_variable) = field.skip_if {
                    if variable_values
                        .as_ref()
                        .and_then(|vars| vars.get(skip_variable))
                        == Some(&Value::Bool(true))
                    {
                        return None;
                    }
                }
                if let Some(ref include_variable) = field.include_if {
                    if variable_values
                        .as_ref()
                        .and_then(|vars| vars.get(include_variable))
                        != Some(&Value::Bool(true))
                    {
                        return None;
                    }
                }
                Some((
                    field.alias.as_ref().unwrap_or(&field.name).to_string(),
                    item,
                ))
            } else {
                None
            }
        })
        .collect();

    obj.retain(|key, value| {
        if let Some(SelectionItem::Field(field_selection)) = selected_fields.get(key) {
            let field_type_name = field_map
                .and_then(|fm| fm.get(&field_selection.name))
                .map(|s| s.as_str());

            if let Some(field_type_name) = field_type_name {
                project_selection_set(
                    value,
                    errors,
                    &field_selection.selections,
                    field_type_name,
                    schema_metadata,
                    variable_values,
                );
            }
            true
        } else {
            key == TYPENAME_FIELD
        }
    });

    for selection in &selection_set.items {
        if let SelectionItem::InlineFragment(inline_fragment) = selection {
            if entity_satisfies_type_condition(
                &schema_metadata.possible_types,
                &type_name,
                &inline_fragment.type_condition,
            ) {
                project_selection_set_with_map(
                    obj,
                    errors,
                    &inline_fragment.selections,
                    &type_name,
                    schema_metadata,
                    variable_values,
                );
            }
        }
    }
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
    if selection_set.items.is_empty() {
        return;
    }
    match data {
        Value::Object(obj) => {
            project_selection_set_with_map(
                obj,
                errors,
                selection_set,
                type_name,
                schema_metadata,
                variable_values,
            );
        }
        Value::Array(arr) => {
            for item in arr {
                project_selection_set(
                    item,
                    errors,
                    selection_set,
                    type_name,
                    schema_metadata,
                    variable_values,
                );
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
    let mut result_data = Value::Null;
    let mut result_errors = vec![];
    let result_extensions = HashMap::new();

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

    result_errors = execution_context.errors;

    if !result_data.is_null() || has_introspection {
        if result_data.is_null() {
            result_data = Value::Object(serde_json::Map::new());
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
        extensions: if execution_context.extensions.is_empty() {
            None
        } else {
            Some(execution_context.extensions)
        },
    }
}

#[cfg(test)]
mod tests;
