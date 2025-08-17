use std::collections::HashMap;

use async_trait::async_trait;
use bytes::Bytes;
use sonic_rs::JsonContainerTrait;

use crate::{
    executors::common::{SubgraphExecutionRequest, SubgraphExecutor},
    response::graphql_error::{GraphQLError, GraphQLErrorLocation, GraphQLErrorPathSegment},
};

#[async_trait]
impl<Executor> SubgraphExecutor for Executor
where
    Executor: async_graphql::Executor,
{
    async fn execute<'a>(&self, execution_request: SubgraphExecutionRequest<'a>) -> Bytes {
        let response: async_graphql::Response = self.execute(execution_request.into()).await;
        serde_json::to_vec(&response).unwrap().into()
    }
}

impl<'a> From<SubgraphExecutionRequest<'a>> for async_graphql::Request {
    fn from(exec_request: SubgraphExecutionRequest) -> Self {
        let mut req = async_graphql::Request::new(exec_request.query);
        if let Some(variables) = exec_request.variables {
            req = req.variables(async_graphql::Variables::from_json(serde_json::json!(
                variables
            )));
        }
        if let Some(representations) = exec_request.representations {
            req.variables.insert(
                async_graphql::Name::new("representations"),
                async_graphql::Value::from_json(
                    serde_json::from_slice(&representations).unwrap_or_default(),
                )
                .unwrap(),
            );
        }
        if let Some(operation_name) = exec_request.operation_name {
            req = req.operation_name(operation_name);
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
                            GraphQLErrorPathSegment::String(name.to_string())
                        }
                        async_graphql::PathSegment::Index(index) => {
                            GraphQLErrorPathSegment::Index((*index).into())
                        }
                    })
                    .collect(),
            ),
            extensions: error.extensions.as_ref().map(|ext| {
                let serialized = sonic_rs::json!(ext);
                serialized
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect::<HashMap<String, sonic_rs::Value>>()
            }),
        }
    }
}
