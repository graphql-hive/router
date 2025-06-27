use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::json;
use sonic_rs::{json as sonic_json, JsonContainerTrait, Value as SonicValue}

use crate::{
    executors::common::SubgraphExecutor, ExecutionRequest, ExecutionResult, GraphQLError,
    GraphQLErrorLocation,
};

#[async_trait]
impl<Executor> SubgraphExecutor for Executor
where
    Executor: async_graphql::Executor,
{
    async fn execute(&self, execution_request: ExecutionRequest) -> ExecutionResult {
        let response: async_graphql::Response = self.execute(execution_request.into()).await;
        response.into()
    }
}

impl From<ExecutionRequest> for async_graphql::Request {
    fn from(exec_request: ExecutionRequest) -> Self {
        let mut req = async_graphql::Request::new(exec_request.query);
        if let Some(variables) = exec_request.variables {
            req = req.variables(async_graphql::Variables::from_json(json!(variables)));
        }
        if let Some(representations) = exec_request.representations {
            req.variables.insert(
                async_graphql::Name::new("representations"),
                async_graphql::Value::from_json(
                    sonic_rs::from_str(&representations).unwrap_or_default(),
                )
                .unwrap(),
            );
        }
        if let Some(operation_name) = exec_request.operation_name {
            req = req.operation_name(operation_name);
        }
        if let Some(extensions) = exec_request.extensions {
            for (key, value) in extensions {
                req.extensions.insert(
                    key,
                    async_graphql::Value::from_json(json!(sonic_rs::to_string(value)))
                        .unwrap_or_default(),
                );
            }
        }
        req
    }
}

impl From<&async_graphql::ServerError> for GraphQLError {
    fn from(error: &async_graphql::ServerError) -> Self {
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
                        async_graphql::PathSegment::Field(name) => SonicValue::from(name),
                        async_graphql::PathSegment::Index(index) => {
                            SonicValue::from(*index)
                        }
                    })
                    .collect(),
            ),
            extensions: error.extensions.as_ref().map(|ext| {
                let serialized = sonic_json!(ext);
                serialized
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect::<HashMap<String, SonicValue>>()
            }),
        }
    }
}

impl From<async_graphql::Response> for ExecutionResult {
    fn from(response: async_graphql::Response) -> Self {
        ExecutionResult {
            data: Some(sonic_json!(serde_json::to_string(response.data.into_json().unwrap()))),
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
                    .collect::<HashMap<String, SonicValue>>(),
            ),
        }
    }
}
