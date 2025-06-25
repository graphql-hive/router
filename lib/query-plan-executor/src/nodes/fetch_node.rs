use std::collections::BTreeMap;

use futures::future::BoxFuture;
use query_planner::planner::plan_nodes::FetchNode;
use serde_json::Value;
use tracing::{instrument, trace};

use crate::{
    execution_context::ExecutionContext,
    execution_request::ExecutionRequest,
    execution_result::ExecutionResult,
    fetch_rewrites::ApplyFetchRewrite,
    projection::SelectionSetProjection,
    traverse_path::{self, SetPathValue, TraversedPathSegment},
};

type TraversedPath = Vec<TraversedPathSegment>;
type VariablesResult = Option<(BTreeMap<String, Value>, Option<Vec<TraversedPath>>)>;
pub trait ExecutableFetchNode {
    fn variables(&self, root: &Value, path: Vec<String>, ctx: &ExecutionContext)
        -> VariablesResult;
    fn variables_from_usages(&self, ctx: &ExecutionContext) -> Option<BTreeMap<String, Value>>;
    fn representations(
        &self,
        root: &Value,
        path: Vec<String>,
        ctx: &ExecutionContext,
    ) -> Option<(Vec<Value>, Vec<TraversedPath>)>;
    fn execute<'a>(
        &'a self,
        root: &'a Value,
        path: Vec<String>,
        ctx: &'a ExecutionContext,
    ) -> BoxFuture<'a, ExecutionResult>;
}

impl ExecutableFetchNode for FetchNode {
    #[instrument(
        level = "debug",
        skip_all,
        name="FetchNode::variables",
        fields(
            path = ?path,
            service_name = self.service_name,
        )
    )]
    fn variables(
        &self,
        root: &Value,
        path: Vec<String>,
        ctx: &ExecutionContext,
    ) -> VariablesResult {
        let representations_and_paths = self.representations(root, path, ctx);
        let variables = self.variables_from_usages(ctx);
        match (representations_and_paths, variables) {
            (None, None) => None, // No representations and no variables
            (Some((representations, paths)), None) => {
                // Only representations available, return them as variables
                let mut map = BTreeMap::new();
                map.insert("representations".to_string(), Value::Array(representations));
                Some((map, Some(paths)))
            }
            (None, Some(variables)) => Some((variables, None)), // Only variables available
            (Some((representations, paths)), Some(mut variables)) => {
                variables.insert("representations".to_string(), Value::Array(representations)); // Merge representations into variables
                Some((variables, Some(paths))) // Both representations and variables available, merge them
            }
        }
    }
    fn variables_from_usages(&self, ctx: &ExecutionContext) -> Option<BTreeMap<String, Value>> {
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
    ) -> Option<(Vec<Value>, Vec<TraversedPath>)> {
        self.requires.as_ref()?;
        let requires = self.requires.as_ref().unwrap();
        if requires.is_empty() {
            return None;
        }
        let mut representations = Vec::new();
        let mut paths = Vec::new();
        let path_slice = path.iter().map(|s| s.as_str()).collect::<Vec<&str>>();
        traverse_path::traverse_path(
            root,
            Vec::with_capacity(0),
            &path_slice,
            &mut |path, entity| {
                let mut projected = requires.project_for_requires(entity, ctx.schema_metadata);
                if !projected.is_null() {
                    if let Some(input_rewrites) = &self.input_rewrites {
                        for rewrite in input_rewrites {
                            rewrite.apply(ctx.schema_metadata, &mut projected);
                        }
                    }
                    representations.push(projected);
                    paths.push(path)
                }
            },
        );
        if representations.is_empty() {
            return None; // No valid representations found
        }
        trace!(
            "Found representations for FetchNode {:?} on path {:?}",
            representations.len(),
            path,
        );
        Some((representations, paths))
    }
    #[instrument(
        level = "debug",
        skip_all,
        name="FetchNode::execute",
        fields(
            service_name = self.service_name,
            operation_name = ?self.operation_name,
            operation_str = %self.operation.operation_str,
            path = ?path,
        )
    )]
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
            if let (Some(output_rewrites), Some(data)) =
                (&self.output_rewrites, &mut subgraph_result.data)
            {
                for rewrite in output_rewrites {
                    rewrite.apply(ctx.schema_metadata, data);
                }
            }
            if let Some(mut paths) = paths {
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
                if let Some(mut entities) = entities {
                    for (path, entity) in paths.drain(..).zip(entities.drain(..)) {
                        final_data.set_path_value(&path, entity);
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
