use std::collections::VecDeque;

use futures::future::BoxFuture;
use query_planner::planner::plan_nodes::FetchNode;
use serde_json::{Map, Value};

use crate::{
    execution_context::ExecutionContext,
    execution_result::ExecutionResult,
    projection::SelectionSetProjection,
    traverse_path::{self, SetPathValue, TraversedPathSegment},
    ExecutionRequest,
};

pub trait ExecutableFetchNode {
    fn variables(
        &self,
        root: &Value,
        path: Vec<String>,
        ctx: &ExecutionContext,
    ) -> Option<(
        Map<String, Value>,
        Option<VecDeque<Vec<TraversedPathSegment>>>,
    )>;
    fn variables_from_usages(&self, ctx: &ExecutionContext) -> Option<Map<String, Value>>;
    fn representations(
        &self,
        root: &Value,
        path: Vec<String>,
        ctx: &ExecutionContext,
    ) -> Option<(VecDeque<Value>, VecDeque<Vec<TraversedPathSegment>>)>;
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext,
    ) -> BoxFuture<'a, ExecutionResult>;
}

impl ExecutableFetchNode for FetchNode {
    fn variables(
        &self,
        root: &Value,
        path: Vec<String>,
        ctx: &ExecutionContext,
    ) -> Option<(
        Map<String, Value>,
        Option<VecDeque<Vec<TraversedPathSegment>>>,
    )> {
        let representations_and_paths = self.representations(root, path, ctx);
        let variables = self.variables_from_usages(ctx);
        match (representations_and_paths, variables) {
            (None, None) => None, // No representations and no variables
            (Some((representations, paths)), None) => {
                // Only representations available, return them as variables
                let mut map = Map::with_capacity(1);
                map.insert(
                    "representations".to_string(),
                    Value::Array(representations.into()),
                );
                Some((map, Some(paths)))
            }
            (None, Some(variables)) => Some((variables, None)), // Only variables available
            (Some((representations, paths)), Some(mut variables)) => {
                variables.insert(
                    "representations".to_string(),
                    Value::Array(representations.into()),
                ); // Merge representations into variables
                Some((variables, Some(paths))) // Both representations and variables available, merge them
            }
        }
    }
    fn variables_from_usages(&self, ctx: &ExecutionContext) -> Option<Map<String, Value>> {
        match (&self.variable_usages, ctx.variables) {
            (None, _) => None, // No variable usages
            (Some(variable_usages), None) => Some(
                variable_usages
                    .iter()
                    .map(|key| (key.to_string(), Value::Null))
                    .collect(),
            ), // No variables defined, return empty map
            (Some(ref variable_usages), Some(variables)) => Some(
                variable_usages
                    .iter()
                    .map(|variable_usage| match variables.get(variable_usage) {
                        Some(value) => (variable_usage.to_string(), value.clone()),
                        None => (variable_usage.to_string(), Value::Null), // If variable not found, use Null
                    })
                    .collect(),
            ),
        }
    }
    fn representations(
        &self,
        root: &Value,
        path: Vec<String>,
        ctx: &ExecutionContext,
    ) -> Option<(VecDeque<Value>, VecDeque<Vec<TraversedPathSegment>>)> {
        self.requires.as_ref()?;
        let requires = self.requires.as_ref().unwrap();
        if requires.is_empty() {
            return None;
        }
        let mut representations = VecDeque::new();
        let mut paths = VecDeque::new();
        let path_slice = path.iter().map(|s| s.as_str()).collect::<Vec<&str>>();
        traverse_path::traverse_path(
            root,
            Vec::with_capacity(0),
            &path_slice,
            &mut |path, entity| {
                let projected = requires.project_for_requires(entity, ctx.schema_metadata);
                if !projected.is_null() {
                    representations.push_back(projected);
                    paths.push_back(path)
                }
            },
        );
        if representations.is_empty() {
            return None; // No valid representations found
        }
        Some((representations, paths))
    }
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext<'_>,
    ) -> BoxFuture<'a, ExecutionResult> {
        Box::pin(async move {
            let variables_and_paths = self.variables(root, path, ctx);
            let (variables, paths) = match variables_and_paths {
                Some((variables, paths)) => (Some(variables), paths),
                None => (None, None), // No variables or representations to execute
            };

            let execution_request = ExecutionRequest {
                query: self.operation.operation_str.clone(),
                operation_name: self.operation_name.clone(),
                variables,
                extensions: None,
            };

            let mut subgraph_result = ctx
                .subgraph_executor_map
                .execute(&self.service_name, execution_request)
                .await;
            // TODO: Output rewrites
            if let Some(paths) = paths {
                let mut final_data = Value::Null;
                let entities = subgraph_result.data.as_mut().and_then(|data| {
                    if let Value::Object(ref mut map) = data {
                        map.remove("_entities").and_then(|entities| {
                            if let Value::Array(entities_array) = entities {
                                Some(entities_array)
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    }
                });
                if let Some(entities) = entities {
                    for (path, entity) in paths.iter().zip(entities.into_iter()) {
                        final_data.set_path_value(path, entity);
                    }
                }
                ExecutionResult::new(
                    Some(final_data),
                    subgraph_result.errors,
                    subgraph_result.extensions,
                )
            } else {
                subgraph_result
            }
        })
    }
}
