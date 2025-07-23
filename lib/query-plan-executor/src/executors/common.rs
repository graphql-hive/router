use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{GraphQLError, SubgraphExecutionRequest};

#[async_trait]
pub trait SubgraphExecutor {
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
    ) -> SubgraphExecutionResult;
    fn to_boxed_arc<'a>(self) -> Arc<Box<dyn SubgraphExecutor + Send + Sync + 'a>>
    where
        Self: Sized + Send + Sync + 'a,
    {
        Arc::new(Box::new(self))
    }
}

pub type SubgraphExecutorType = dyn crate::executors::common::SubgraphExecutor + Send + Sync;

pub type SubgraphExecutorBoxedArc = Arc<Box<SubgraphExecutorType>>;

#[derive(Deserialize, Debug, Clone, Default)]
pub struct SubgraphExecutionResultData {
    pub _entities: Option<Vec<Value>>,
    #[serde(flatten)]
    pub root_fields: Map<String, Value>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct SubgraphExecutionResult {
    pub data: Option<SubgraphExecutionResultData>,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<HashMap<String, Value>>,
}

impl SubgraphExecutionResult {
    pub fn from_error_message(message: String) -> SubgraphExecutionResult {
        SubgraphExecutionResult {
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
