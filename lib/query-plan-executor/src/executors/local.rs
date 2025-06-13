use std::collections::HashMap;

use async_graphql::{dynamic::Schema, PathSegment, Response, ServerError};
use async_trait::async_trait;

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
        async_graphql::Request::new(exec_request.query)
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
            // TODO: Extensions
            extensions: None,
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
