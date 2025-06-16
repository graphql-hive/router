use std::collections::HashMap;

use async_graphql::{dynamic::Schema, PathSegment, Response, ServerError, Variables};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    executors::common::SubgraphExecutor, ExecutionRequest, ExecutionResult, GraphQLError,
    GraphQLErrorLocation,
};

pub struct LocalSubgraphExecutor<'a> {
    pub subgraph_schema_map: &'a HashMap<String, Schema>,
}

#[async_trait]
impl SubgraphExecutor for LocalSubgraphExecutor<'_> {
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: crate::ExecutionRequest,
    ) -> crate::ExecutionResult {
        match self.subgraph_schema_map.get(subgraph_name) {
            Some(schema) => {
                let response: Response = schema.execute(execution_request).await;
                response.into()
            }
            None => crate::ExecutionResult::from_error_message(format!(
                "Subgraph {} not found in schema map",
                subgraph_name
            )),
        }
    }
}

impl From<ExecutionRequest> for async_graphql::Request {
    fn from(exec_request: ExecutionRequest) -> Self {
        let mut req = async_graphql::Request::new(exec_request.query);
        if let Some(variables) = exec_request.variables {
            req = req.variables(Variables::from_json(json!(variables)));
        }
        if let Some(operation_name) = exec_request.operation_name {
            req = req.operation_name(operation_name);
        }
        if let Some(extensions) = exec_request.extensions {
            for (key, value) in extensions {
                req.extensions.insert(
                    key,
                    async_graphql::Value::from_json(value).unwrap_or_default(),
                );
            }
        }
        req
    }
}

impl From<&ServerError> for GraphQLError {
    fn from(error: &ServerError) -> Self {
        GraphQLError {
            message: error.message.to_string(),
            locations: Some(
                error
                    .locations
                    .iter()
                    .map(|loc| GraphQLErrorLocation {
                        line: loc.line,
                        column: loc.column,
                    })
                    .collect(),
            ),
            path: Some(
                error
                    .path
                    .iter()
                    .map(|s| match s {
                        PathSegment::Field(name) => serde_json::Value::String(name.to_string()),
                        PathSegment::Index(index) => serde_json::Value::Number((*index).into()),
                    })
                    .collect(),
            ),
            extensions: error.extensions.as_ref().map(|ext| {
                let serialized = json!(ext);
                serialized
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect::<HashMap<String, serde_json::Value>>()
            }),
        }
    }
}

impl From<Response> for ExecutionResult {
    fn from(response: Response) -> Self {
        ExecutionResult {
            data: Some(response.data.into_json().unwrap()),
            errors: Some(
                response
                    .errors
                    .iter()
                    .map(|error| error.into())
                    .collect::<Vec<GraphQLError>>(),
            ),
            extensions: Some(
                response
                    .extensions
                    .into_iter()
                    .map(|(key, value)| (key, value.into_json().unwrap()))
                    .collect::<HashMap<String, serde_json::Value>>(),
            ),
        }
    }
}
