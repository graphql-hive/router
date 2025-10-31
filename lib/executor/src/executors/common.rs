use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
use http::HeaderMap;
use sonic_rs::Value;

use crate::execution::client_request_details::ClientRequestDetails;

#[async_trait]
pub trait SubgraphExecutor {
    async fn execute<'exec, 'req>(
        &self,
        execution_request: HttpExecutionRequest<'exec, 'req>,
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

pub type SubgraphRequestExtensions = HashMap<String, Value>;

pub struct HttpExecutionRequest<'exec, 'req> {
    pub query: &'exec str,
    pub dedupe: bool,
    pub operation_name: Option<&'exec str>,
    // TODO: variables could be stringified before even executing the request
    pub variables: Option<HashMap<&'exec str, &'exec sonic_rs::Value>>,
    pub headers: HeaderMap,
    pub representations: Option<Vec<u8>>,
    pub extensions: Option<SubgraphRequestExtensions>,
    pub client_request: &'exec ClientRequestDetails<'exec, 'req>,
}

impl HttpExecutionRequest<'_, '_> {
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
