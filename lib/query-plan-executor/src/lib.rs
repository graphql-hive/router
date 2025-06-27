use async_trait::async_trait;
use futures::future::BoxFuture;
use query_planner::{
    ast::{operation::OperationDefinition, selection_item::SelectionItem},
    planner::plan_nodes::{
        ConditionNode, FetchNode, FetchNodePathSegment, FetchRewrite, FlattenNode, FlattenNodePath,
        FlattenNodePathSegment, KeyRenamer, ParallelNode, PlanNode, QueryPlan, SequenceNode,
        ValueSetter,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use tracing::{instrument, trace, warn}; // For reading file in main

use crate::{executors::map::SubgraphExecutorMap, schema_metadata::SchemaMetadata};
pub mod deep_merge;
pub mod executors;
pub mod introspection;
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
        trace!(
            "Processing representations result: {} entities",
            entities.len()
        );
        for (entity, index) in entities.into_iter().zip(result.indexes.into_iter()) {
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
        filtered_representations: String,
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
    representations: String,
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
            operation_str = %self.operation.document_str,
        )
    )]
    async fn execute_for_root(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
    ) -> ExecutionResult {
        let variables = self.prepare_variables_for_fetch_node(execution_context.variable_values);

        let execution_request = ExecutionRequest {
            query: self.operation.document_str.clone(),
            operation_name: self.operation_name.clone(),
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

    fn project_representations(
        &self,
        execution_context: &QueryPlanExecutionContext<'_>,
        representations: &[&mut Value],
    ) -> ProjectRepresentationsResult {
        let mut filtered_repr_indexes = Vec::new();
        // 1. Filter representations based on requires (if present)
        let mut filtered_representations = vec![];
        let requires_nodes = self.requires.as_ref().unwrap();
        for (index, entity) in representations.iter().enumerate() {
            let entity_projected =
                execution_context.project_requires(&requires_nodes.items, entity);
            if entity_projected != "null" {
                filtered_representations.push(entity_projected);
                filtered_repr_indexes.push(index);
            }
        }

        // if let Some(input_rewrites) = &self.input_rewrites {
        //     for representation in filtered_representations.iter_mut() {
        //         for rewrite in input_rewrites {
        //             rewrite.apply(
        //                 &execution_context.schema_metadata.possible_types,
        //                 representation,
        //             );
        //         }
        //     }
        // }

        ProjectRepresentationsResult {
            representations: "[".to_string() + &filtered_representations.join(",") + "]",
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
        filtered_representations: String,
        filtered_repr_indexes: Vec<usize>,
    ) -> ExecuteForRepresentationsResult {
        // 2. Prepare variables for fetch
        let execution_request = ExecutionRequest {
            query: self.operation.document_str.clone(),
            operation_name: self.operation_name.clone(),
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
        path: &[FetchNodePathSegment],
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
        path: &[FetchNodePathSegment],
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
                        if entity_satisfies_type_condition(
                            possible_types,
                            type_name,
                            type_condition,
                        ) {
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
    fn apply(&self, possible_types: &HashMap<String, Vec<String>>, data: &mut Value) {
        self.apply_path(possible_types, data, &self.path)
    }

    // Applies value setting on a Value (returns a new Value)
    fn apply_path(
        &self,
        possible_types: &HashMap<String, Vec<String>>,
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
                        if entity_satisfies_type_condition(
                            possible_types,
                            type_name,
                            type_condition,
                        ) {
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

type ExecutionStepJob<'a, T> = BoxFuture<'a, T>;

#[derive(Default)]
struct ExecutionStep<'a> {
    fetch_job: Option<ExecutionStepJob<'a, ExecutionResult>>,
    flatten_job: Option<ExecutionStepJob<'a, ExecuteForRepresentationsResult>>,
    flatten_path: Option<&'a FlattenNodePath>,
}

fn create_execution_step<'a>(
    node: &'a PlanNode,
    execution_context: &'a QueryPlanExecutionContext<'a>,
    data: &mut Value,
) -> ExecutionStep<'a> {
    match node {
        PlanNode::Fetch(fetch_node) => ExecutionStep {
            fetch_job: Some(fetch_node.execute_for_root(execution_context)),
            ..Default::default()
        },
        PlanNode::Flatten(flatten_node) => {
            let PlanNode::Fetch(fetch_node) = flatten_node.node.as_ref() else {
                warn!(
                    "FlattenNode can only execute FetchNode as child node, found: {:?}",
                    flatten_node.node
                );
                return ExecutionStep::default();
            };

            let collected_representations =
                traverse_and_collect(data, flatten_node.path.as_slice());

            if collected_representations.is_empty() {
                // No representations collected, skip execution
                return ExecutionStep::default();
            }

            let project_result =
                fetch_node.project_representations(execution_context, &collected_representations);

            if project_result.representations.is_empty() {
                // No representations collected, skip execution
                return ExecutionStep::default();
            }

            let job = fetch_node.execute_for_projected_representations(
                execution_context,
                project_result.representations,
                project_result.indexes,
            );

            ExecutionStep {
                flatten_job: Some(job),
                flatten_path: Some(&flatten_node.path),
                ..Default::default()
            }
        }
        PlanNode::Condition(node) => {
            let condition_value = execution_context
                .variable_values
                .as_ref()
                .and_then(|vars| vars.get(&node.condition))
                .is_some_and(|val| match val {
                    Value::Bool(b) => *b,
                    _ => false,
                });

            let clause = if condition_value {
                node.if_clause.as_deref()
            } else {
                node.else_clause.as_deref()
            };

            if let Some(clause) = clause {
                create_execution_step(clause, execution_context, data)
            } else {
                ExecutionStep::default()
            }
        }
        _ => ExecutionStep::default(),
    }
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
        // Here we call fetch nodes in parallel and non-fetch nodes sequentially.
        let mut fetch_jobs = vec![];
        let mut flatten_jobs = vec![];
        let mut flatten_paths = vec![];

        // Collect Fetch node results and non-fetch nodes for sequential execution
        let now = std::time::Instant::now();
        for node in &self.nodes {
            let res = create_execution_step(node, execution_context, data);

            if let Some(fetch_job) = res.fetch_job {
                fetch_jobs.push(fetch_job);
            }
            if let Some(flatten_job) = res.flatten_job {
                flatten_jobs.push(flatten_job);
            }
            if let Some(flatten_path) = res.flatten_path {
                flatten_paths.push(flatten_path.as_slice());
            }
        }
        trace!(
            "Prepared {} fetch jobs and {} flatten jobs in {:?}",
            fetch_jobs.len(),
            flatten_jobs.len(),
            now.elapsed()
        );

        let mut all_errors = vec![];
        let mut all_extensions = vec![];

        let now = std::time::Instant::now();

        let flatten_results = futures::future::join_all(flatten_jobs).await;
        let flatten_results_len = flatten_results.len();

        trace!(
            "Executed {} flatten jobs in {:?}",
            flatten_results_len,
            now.elapsed()
        );

        let now = std::time::Instant::now();
        for (result, path) in flatten_results.into_iter().zip(flatten_paths) {
            // Process FlattenNode results
            if let Some(entities) = result.entities {
                let mut collected_representations = traverse_and_collect(data, path);
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

        trace!(
            "Processed {} flatten results in {:?}",
            flatten_results_len,
            now.elapsed()
        );

        let now = std::time::Instant::now();
        let fetch_results = futures::future::join_all(fetch_jobs).await;
        let fetch_results_len = fetch_results.len();
        trace!(
            "Executed {} fetch jobs in {:?}",
            fetch_results_len,
            now.elapsed()
        );

        let now = std::time::Instant::now();
        // Process results from FetchNode executions
        for fetch_result in fetch_results {
            process_root_result(fetch_result, execution_context, data);
        }

        trace!(
            "Processed {} fetch results in {:?}",
            fetch_results_len,
            now.elapsed()
        );

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
        let mut representations = traverse_and_collect(data, self.path.as_slice());
        trace!(
            "traversed and collected representations: {:?} in {:#?}",
            representations.len(),
            now.elapsed()
        );

        if representations.is_empty() {
            // If there are no representations,
            // return early without executing the child node.
            return;
        }

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

                if filtered_representations.is_empty() {
                    // If there are no filtered representations,
                    // return early without executing the child node.
                    return;
                }

                let now = std::time::Instant::now();
                let result = fetch_node
                    .execute_for_projected_representations(
                        execution_context,
                        filtered_representations,
                        filtered_repr_indexes,
                    )
                    .await;
                trace!(
                    "executed projected representations: {:?} in {:?}",
                    representations.len(),
                    now.elapsed()
                );
                // Process the result
                process_representations_result(result, &mut representations, execution_context);
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

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequest {
    pub query: String,
    pub operation_name: Option<String>,
    pub variables: Option<HashMap<String, Value>>,
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
    fn project_requires_map(
        &self,
        requires_selections: &Vec<SelectionItem>,
        entity_obj: &Map<String, Value>,
    ) -> Vec<String> {
        let mut items = vec![];
        for requires_selection in requires_selections {
            match &requires_selection {
                SelectionItem::Field(requires_selection) => {
                    let field_name = &requires_selection.name;
                    let response_key = requires_selection.selection_identifier();
                    let original = entity_obj
                        .get(field_name)
                        .unwrap_or(entity_obj.get(response_key).unwrap_or(&Value::Null));
                    let projected_value =
                        self.project_requires(&requires_selection.selections.items, original);
                    if projected_value != "null" && !projected_value.is_empty() {
                        items.push("\"".to_string() + response_key + "\":" + &projected_value)
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
                            .project_requires_map(&requires_selection.selections.items, entity_obj);
                        // Merge the projected value into the result
                        if !projected.is_empty() {
                            items.extend(projected);
                        }
                        // If the projected value is not an object, it will be ignored
                    }
                }
            }
        }
        items
    }
    pub fn project_requires(
        &self,
        requires_selections: &Vec<SelectionItem>,
        entity: &Value,
    ) -> String {
        if requires_selections.is_empty() {
            return serde_json::to_string(entity).unwrap(); // No selections to project, return the entity as is
        }
        match entity {
            Value::Null => "null".to_string(),
            Value::Array(entity_array) => {
                let mut items = Vec::with_capacity(entity_array.len());
                for entity_item in entity_array {
                    let projected_value = self.project_requires(requires_selections, entity_item);
                    if projected_value != "null" && !projected_value.is_empty() {
                        items.push(projected_value);
                    }
                }
                "[".to_string() + &items.join(",") + "]"
            }
            Value::Object(entity_obj) => {
                let items = self.project_requires_map(requires_selections, entity_obj);

                if items.is_empty() {
                    return "null".to_string(); // No items to project, return null
                }

                // Join the items into a JSON object string
                let projected_string = items.join(",");
                "{".to_string() + &projected_string + "}"
            }
            Value::Bool(false) => "false".to_string(),
            Value::Bool(true) => "true".to_string(),
            Value::Number(num) => num.to_string(),
            Value::String(string) => serde_json::to_string(string).unwrap(),
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

/// Recursively traverses the data according to the path segments,
/// handling '@' for array iteration, and collects the final values.current_data.to_vec()
#[instrument(level = "trace", skip_all, fields(
    current_type = ?current_data,
    remaining_path = ?remaining_path
))]
pub fn traverse_and_collect<'a>(
    current_data: &'a mut Value,
    remaining_path: &[FlattenNodePathSegment],
) -> Vec<&'a mut Value> {
    let mut collected = Vec::new();
    traverse_and_collect_mut(current_data, remaining_path, &mut collected);
    collected
}

fn traverse_and_collect_mut<'a>(
    current_data: &'a mut Value,
    remaining_path: &[&str],
    collected: &mut Vec<&'a mut Value>,
) {
    if remaining_path.is_empty() {
        collected.push(current_data);
        return;
    }

    let key = remaining_path[0];
    let rest_of_path = &remaining_path[1..];

    if key == "@" {
        if let Value::Array(list) = current_data {
            for item in list.iter_mut() {
                traverse_and_collect_mut(item, rest_of_path, collected);
            }
        }
    } else if let Value::Object(map) = current_data {
        if let Some(next_data) = map.get_mut(key) {
            traverse_and_collect_mut(next_data, rest_of_path, collected);
        }
    }
}

/// Checks if a serde_json::Value has a `__typename` that matches the given `type_name`.
fn contains_typename(value: &Value, type_name: &str, default_for_missing: bool) -> bool {
    value
        .as_object()
        .and_then(|obj| obj.get("__typename"))
        .and_then(Value::as_str)
        .map_or(default_for_missing, |s| s == type_name)
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
