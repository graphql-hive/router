use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
use http::HeaderMap;

use crate::execution::plan::ClientRequestDetails;

#[async_trait]
pub trait SubgraphExecutor {
    async fn execute<'a>(
        &self,
        execution_request: HttpExecutionRequest<'a>,
    ) -> HttpExecutionResponse;
    fn to_boxed_arc<'a>(self) -> Arc<Box<dyn SubgraphExecutor + Send + Sync + 'a>>
    where
        Self: Sized + Send + Sync + 'a,
    {
        Arc::new(Box::new(self))
    }
}

pub type SubgraphExecutorType = dyn crate::executors::common::SubgraphExecutor + Send + Sync;

pub type SubgraphExecutorBoxedArc = Arc<Box<SubgraphExecutorType>>;

pub struct HttpExecutionRequest<'a> {
    pub query: &'a str,
    pub dedupe: bool,
    pub operation_name: Option<&'a str>,
    // TODO: variables could be stringified before even executing the request
    pub variables: Option<HashMap<&'a str, &'a sonic_rs::Value>>,
    pub headers: HeaderMap,
    pub representations: Option<Vec<u8>>,
    pub client_request: &'a ClientRequestDetails<'a>,
}

pub struct HttpExecutionResponse {
    pub body: Bytes,
    pub headers: HeaderMap,
}
