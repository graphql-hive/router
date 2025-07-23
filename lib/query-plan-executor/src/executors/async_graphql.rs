use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::{
    executors::common::{SubgraphExecutionResult, SubgraphExecutionResultData, SubgraphExecutor},
    GraphQLError, GraphQLErrorLocation, SubgraphExecutionRequest,
};

#[async_trait]
impl<Executor> SubgraphExecutor for Executor
where
    Executor: async_graphql::Executor,
{
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
    ) -> SubgraphExecutionResult {
        let response: async_graphql::Response = self.execute(execution_request.into()).await;
        response.into()
    }
}

impl<'a> From<SubgraphExecutionRequest<'a>> for async_graphql::Request {
    fn from(exec_request: SubgraphExecutionRequest) -> Self {
        let mut req = async_graphql::Request::new(exec_request.query);
        if let Some(variables) = exec_request.variables {
            req = req.variables(async_graphql::Variables::from_json(json!(variables)));
        }
        if let Some(representations) = exec_request.representations {
            req.variables.insert(
                async_graphql::Name::new("representations"),
                async_graphql::Value::from_json(
                    sonic_rs::from_slice(&representations).unwrap_or_default(),
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
                    async_graphql::Value::from_json(value).unwrap_or_default(),
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
                        async_graphql::PathSegment::Field(name) => {
                            serde_json::Value::String(name.to_string())
                        }
                        async_graphql::PathSegment::Index(index) => {
                            serde_json::Value::Number((*index).into())
                        }
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

impl From<async_graphql::Response> for SubgraphExecutionResult {
    fn from(response: async_graphql::Response) -> Self {
        SubgraphExecutionResult {
            data: SubgraphExecutionResultData::deserialize(response.data.into_json().unwrap())
                .map(Some)
                .unwrap_or(None),
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
