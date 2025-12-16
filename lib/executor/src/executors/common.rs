use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use http::HeaderMap;
use sonic_rs::Value;

#[async_trait]
pub trait SubgraphExecutor {
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> HttpExecutionResponse;

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> BoxStream<
        'static,
        // the stream technically yields execution responses
        // we use the same one because I think it's ok to have
        // every event also include the headers for easier
        // plugin lifecycle design?
        HttpExecutionResponse,
    >;

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

pub struct HttpExecutionResponse {
    pub body: Bytes,
    pub headers: HeaderMap,
}
