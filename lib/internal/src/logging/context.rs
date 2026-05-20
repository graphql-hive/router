use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use hive_router_config::{
    log::service::{CorrelationConfig, LogFieldsConfig},
    primitives::http_header::HttpHeaderName,
};
use ntex::http::body::{BodySize, MessageBody};
use tracing::info;

use crate::logging::{request_id::RequestIdentifierExtractor, sonic_valuable::SonicMapRef};

// The following are defined as consts in order to make it easier to change, and also to make it
// accessible to testing infrastructure.
pub const LOG_HTTP_REQUEST_START: &str = "http request started";
pub const LOG_HTTP_REQUEST_COMPLETED: &str = "http request completed";
pub const LOG_GRAPHQL_REQUEST_START: &str = "graphql request started";
pub const LOG_GRAPHQL_REQUEST_COMPLETED: &str = "graphql request completed";
pub const LOG_SUBGRAPH_REQUEST_START: &str = "subgraph request started";
pub const LOG_SUBGRAPH_REQUEST_COMPLETED: &str = "subgraph request completed";

#[derive(Clone, Default)]
pub struct LoggerContext {
    fields_config: LogFieldsConfig,
    pub identifier_extractor: RequestIdentifierExtractor,
}

impl LoggerContext {
    pub fn new(fields_config: LogFieldsConfig, correlation_config: CorrelationConfig) -> Self {
        Self {
            fields_config,
            identifier_extractor: RequestIdentifierExtractor::new(correlation_config),
        }
    }

    #[inline]
    pub fn http_request_start(&self, request: &ntex::web::HttpRequest) {
        let cfg = &self.fields_config.http.request;
        let method = cfg.method.then(|| request.method().as_str());
        let path = cfg.path.then(|| request.path());
        let query_string = cfg.query_string.then(|| request.query_string());
        let headers =
            (!cfg.headers.is_empty()).then(|| obtain_headers(request.headers(), &cfg.headers));

        info!(
            target: "http.server",
            method,
            path,
            query_string,
            headers = tracing::field::valuable(&headers),
            "{}",
            LOG_HTTP_REQUEST_START
        );
    }

    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn graphql_request_start(
        &self,
        body_size: usize,
        client_name: Option<&str>,
        client_version: Option<&str>,
        query: Option<&str>,
        operation_name: Option<&str>,
        variables: &HashMap<String, sonic_rs::Value>,
        extensions: Option<&HashMap<String, sonic_rs::Value>>,
    ) {
        let cfg = &self.fields_config.graphql.request;
        let body_size_bytes = cfg.body_size_bytes.then_some(body_size as i64);
        let client_name = cfg.client_name.then_some(client_name);
        let client_version = cfg.client_version.then_some(client_version);
        let operation = cfg.operation.then_some(query);
        let operation_name = cfg.operation_name.then_some(operation_name.unwrap_or(""));
        let variables = cfg.variables.then_some(SonicMapRef(variables));
        let extensions = cfg.extensions.then(|| extensions.map(SonicMapRef));

        info!(
            target: "graphql.engine",
            body_size_bytes,
            client_name,
            client_version,
            operation,
            operation_name,
            variables = tracing::field::valuable(&variables),
            extensions = tracing::field::valuable(&extensions),
            "{}",
            LOG_GRAPHQL_REQUEST_START
        );
    }

    #[inline]
    pub fn subgraph_request_start(
        &self,
        subgraph_name: &str,
        operation: &str,
        operation_name: Option<&str>,
        variables: Option<&HashMap<&str, &sonic_rs::Value>>,
    ) {
        let cfg = &self.fields_config.subgraph.request;
        let operation = cfg.operation.then_some(operation);
        let operation_name = cfg.operation_name.then_some(operation_name.unwrap_or(""));
        let variables = cfg.variables.then(|| variables.map(SonicMapRef));

        info!(
            target: "graphql.executor",
            subgraph = subgraph_name,
            operation,
            operation_name,
            variables = tracing::field::valuable(&variables),
            "{}",
            LOG_SUBGRAPH_REQUEST_START
        );
    }

    #[inline]
    pub fn subgraph_request_end(&self, subgraph_name: &str, error_count: usize) {
        let cfg = &self.fields_config.subgraph.response;
        let error_count = cfg.error_count.then_some(error_count);

        info!(
            target: "graphql.executor",
            subgraph = subgraph_name,
            error_count, "{}", LOG_SUBGRAPH_REQUEST_COMPLETED
        );
    }

    #[inline]
    pub fn graphql_request_end(&self, error_count: usize) {
        let error_count = self
            .fields_config
            .graphql
            .response
            .error_count
            .then_some(error_count);

        info!(target: "graphql.engine", error_count, "{}", LOG_GRAPHQL_REQUEST_COMPLETED);
    }

    #[inline]
    pub fn http_request_end(&self, duration: Duration, response: &ntex::http::Response) {
        let cfg = &self.fields_config.http.response;
        let status_code = cfg.status_code.then_some(response.status().as_u16());
        let duration_ms = cfg.duration_ms.then_some(duration.as_millis());
        let headers =
            (!cfg.headers.is_empty()).then(|| obtain_headers(response.headers(), &cfg.headers));
        let payload_bytes = cfg.payload_bytes.then(|| match response.body().size() {
            BodySize::Empty | BodySize::None => 0,
            BodySize::Sized(size) => size as i64,
            BodySize::Stream => -1,
        });

        info!(
            target: "http.server",
            status_code,
            duration_ms,
            headers = tracing::field::valuable(&headers),
            payload_bytes,
            "{}",
            LOG_HTTP_REQUEST_COMPLETED
        );
    }
}

#[inline]
fn obtain_headers<'req, 'cfg>(
    header_map: &'req ntex::http::HeaderMap,
    headers_list: &'cfg [HttpHeaderName],
) -> BTreeMap<&'cfg str, Option<&'req str>> {
    headers_list
        .iter()
        .map(|header| {
            let header_str = header.get_header_ref();
            let value = header_map
                .get(header_str)
                .and_then(|value| value.to_str().ok());

            (header_str.as_str(), value)
        })
        .collect()
}
