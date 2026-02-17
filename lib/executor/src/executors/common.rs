use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::stream::BoxStream;
use http::HeaderMap;
use sonic_rs::Value;

use crate::{
    executors::error::SubgraphExecutorError, response::subgraph_response::SubgraphResponse,
};

#[async_trait]
pub trait SubgraphExecutor {
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> Result<SubgraphResponse<'a>, SubgraphExecutorError>;

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> Result<BoxStream<'static, SubgraphResponse<'static>>, SubgraphExecutorError>;

    fn to_boxed_arc<'a>(self) -> Arc<Box<dyn SubgraphExecutor + Send + Sync + 'a>>
    where
        Self: Sized + Send + Sync + 'a,
    {
        Arc::new(Box::new(self))
    }
}

pub type SubgraphExecutorType = dyn crate::executors::common::SubgraphExecutor + Send + Sync;

pub type SubgraphExecutorBoxedArc = Arc<Box<SubgraphExecutorType>>;

pub type SubgraphRequestExtensions = HashMap<String, Value>;

pub struct SubgraphExecutionRequest<'a> {
    pub query: &'a str,
    pub dedupe: bool,
    pub operation_name: Option<&'a str>,
    // TODO: variables could be stringified before even executing the request
    pub variables: Option<HashMap<&'a str, &'a sonic_rs::Value>>,
    pub headers: HeaderMap,
    pub representations: Option<Vec<u8>>,
    pub extensions: Option<SubgraphRequestExtensions>,
}

impl SubgraphExecutionRequest<'_> {
    pub fn add_request_extensions_field(&mut self, key: String, value: Value) {
        self.extensions
            .get_or_insert_with(HashMap::new)
            .insert(key, value);
    }
}
