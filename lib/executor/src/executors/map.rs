use std::{collections::HashMap, sync::Arc, time::Duration};

use bytes::{BufMut, Bytes, BytesMut};
use hyper_util::{
    client::legacy::Client,
    rt::{TokioExecutor, TokioTimer},
};

use crate::{
    executors::{
        common::{HttpExecutionRequest, SubgraphExecutor, SubgraphExecutorBoxedArc},
        error::SubgraphExecutorError,
        http::HTTPSubgraphExecutor,
    },
    response::graphql_error::GraphQLError,
};

pub struct SubgraphExecutorMap {
    inner: HashMap<String, SubgraphExecutorBoxedArc>,
}

impl Default for SubgraphExecutorMap {
    fn default() -> Self {
        Self::new()
    }
}

impl SubgraphExecutorMap {
    pub fn new() -> Self {
        SubgraphExecutorMap {
            inner: HashMap::new(),
        }
    }

    pub async fn execute<'a>(
        &self,
        subgraph_name: &str,
        execution_request: HttpExecutionRequest<'a>,
    ) -> Bytes {
        match self.inner.get(subgraph_name) {
            Some(executor) => executor.execute(execution_request).await,
            None => {
                let graphql_error: GraphQLError = format!(
                    "Subgraph executor not found for subgraph: {}",
                    subgraph_name
                )
                .into();
                let errors = vec![graphql_error];
                let errors_bytes = sonic_rs::to_vec(&errors).unwrap();
                let mut buffer = BytesMut::new();
                buffer.put_slice(b"{\"errors\":");
                buffer.put_slice(&errors_bytes);
                buffer.put_slice(b"}");
                buffer.freeze()
            }
        }
    }

    pub fn insert_boxed_arc(&mut self, subgraph_name: String, boxed_arc: SubgraphExecutorBoxedArc) {
        self.inner.insert(subgraph_name, boxed_arc);
    }

    pub fn from_http_endpoint_map(
        subgraph_endpoint_map: HashMap<String, String>,
    ) -> Result<Self, SubgraphExecutorError> {
        let mut builder = Client::builder(TokioExecutor::new());
        let builder_mut = builder
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(Duration::from_secs(60 * 60))
            .pool_max_idle_per_host(usize::MAX);
        let http_client = builder_mut.build_http();
        let http_client_arc = Arc::new(http_client);
        let executor_map = subgraph_endpoint_map
            .into_iter()
            .map(|(subgraph_name, endpoint)| {
                HTTPSubgraphExecutor::new(&endpoint, http_client_arc.clone())
                    .map(|executor| (subgraph_name, executor.to_boxed_arc()))
            })
            .collect::<Result<HashMap<_, _>, SubgraphExecutorError>>()?;
        Ok(SubgraphExecutorMap {
            inner: executor_map,
        })
    }
}
