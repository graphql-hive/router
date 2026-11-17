use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Duration,
};

use aws_lc_rs::digest;

use crate::executor::response::shaped::ResponseShape;
use crate::query_planner::planner::plan_nodes::CustomScalarPaths;
use async_trait::async_trait;
use futures::stream::BoxStream;
use http::{HeaderMap, Uri};
use sonic_rs::Value;

use crate::executor::{
    executors::error::SubgraphExecutorError, plugin_context::PluginRequestState,
    response::subgraph_response::SubgraphResponse,
};

#[async_trait]
pub trait SubgraphExecutor {
    fn executor_name(&self) -> &str;

    fn endpoint(&self) -> &Uri;

    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: Option<&'a PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'static>, SubgraphExecutorError>;

    async fn subscribe<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
    ) -> Result<
        BoxStream<'static, Result<SubgraphResponse<'static>, SubgraphExecutorError>>,
        SubgraphExecutorError,
    >;

    fn to_boxed_arc<'a>(self) -> Arc<Box<dyn SubgraphExecutor + Send + Sync + 'a>>
    where
        Self: Sized + Send + Sync + 'a,
    {
        Arc::new(Box::new(self))
    }
}

pub type SubgraphExecutorType =
    dyn crate::executor::executors::common::SubgraphExecutor + Send + Sync;

pub type SubgraphExecutorBoxedArc = Arc<Box<SubgraphExecutorType>>;

pub type SubgraphRequestExtensions = HashMap<String, Value>;

/// Identifies the inbound connection context that may be reused across GraphQL operations.
///
/// The hash covers the request method, path, selected inbound headers, and schema checksum. It
/// deliberately excludes operation data, variables, and extensions so different operations from
/// the same authenticated connection context can share an initialized subgraph connection.
///
/// This type can be used as a connection pool key.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConnectionFingerprint(u64);

impl ConnectionFingerprint {
    /// Wraps a hash produced from connection-scoped request inputs.
    pub fn from_hash(hash: u64) -> Self {
        Self(hash)
    }
}

/// Identifies one complete inbound GraphQL request for in-flight request deduplication.
///
/// The hash extends a [`ConnectionFingerprint`] with the normalized operation, variables, and
/// extensions hashes. Identical values may share one execution and its response. Different
/// operations therefore have different request fingerprints even when they are eligible to share
/// the same pooled connection.
///
/// This type is used only for request deduplication and must not be used as a connection pool key.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InboundRequestFingerprint(u64);

impl InboundRequestFingerprint {
    /// Wraps a hash produced from a connection fingerprint and operation-scoped inputs.
    pub fn from_hash(hash: u64) -> Self {
        Self(hash)
    }
}

#[derive(Clone)]
pub struct SubgraphExecutionRequest<'a> {
    /// Holds the query string to be executed.
    /// The query string contains an anonymous GraphQL document.
    /// The name is during execution at `document_name_write_pos` position in the query string.
    pub query: &'a str,
    pub document_name_write_pos: usize,
    pub dedupe: bool,
    pub operation_name: Option<String>,
    // TODO: variables could be stringified before even executing the request
    pub variables: Option<HashMap<&'a str, &'a sonic_rs::Value>>,
    pub headers: HeaderMap,
    pub raw_variable_values: Option<Vec<(&'a str, Vec<u8>)>>,
    pub extensions: Option<SubgraphRequestExtensions>,
    pub custom_scalar_paths: Option<&'a CustomScalarPaths>,
    /// The fields the response has, to read it faster
    pub response_shape: Option<&'a ResponseShape>,
    /// Identifies an existing pooled connection that may execute this request.
    ///
    /// `None` disables pooled lookup and preserves the request's normal transport behavior.
    pub connection_fingerprint: Option<ConnectionFingerprint>,
    /// The plan's query and its cached hash for APQ. Only used while `query` is that query.
    pub query_hash: Option<(&'a str, &'a QueryHash)>,
    /// Lets this request use APQ, if the subgraph has it on. A plugin that rewrites the body in
    /// `on_subgraph_http_request` should turn it off in `on_subgraph_execute`.
    pub apq: bool,
}

/// SHA-256 (hex) of a plan's query, computed on first use.
#[derive(Debug, Clone, Default)]
pub struct QueryHash(OnceLock<Box<[u8; 64]>>);

impl QueryHash {
    pub fn get(&self, query: &str) -> &str {
        let hex = self.0.get_or_init(|| Box::new(sha256_hex_bytes([query])));
        std::str::from_utf8(&hex[..]).expect("hex is ASCII")
    }
}

pub fn sha256_hex<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let hex = sha256_hex_bytes(parts);
    std::str::from_utf8(&hex).expect("hex is ASCII").to_string()
}

fn sha256_hex_bytes<'a>(parts: impl IntoIterator<Item = &'a str>) -> [u8; 64] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut context = digest::Context::new(&digest::SHA256);
    for part in parts {
        context.update(part.as_bytes());
    }
    let mut hex = [0u8; 64];
    for (i, byte) in context.finish().as_ref().iter().enumerate() {
        hex[i * 2] = HEX[(byte >> 4) as usize];
        hex[i * 2 + 1] = HEX[(byte & 0xf) as usize];
    }
    hex
}

impl SubgraphExecutionRequest<'_> {
    pub fn add_request_extensions_field(&mut self, key: String, value: Value) {
        self.extensions
            .get_or_insert_with(HashMap::new)
            .insert(key, value);
    }
}
