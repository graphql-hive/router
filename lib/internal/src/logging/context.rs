use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use hive_router_config::{log::service::LogFieldsConfig, primitives::http_header::HttpHeaderName};
use ntex::http::body::{BodySize, MessageBody};
use tracing::info;

#[derive(Clone, Default)]
pub struct LoggerContext {
    fields_config: LogFieldsConfig,
}

impl LoggerContext {
    pub fn new(fields_config: LogFieldsConfig) -> Self {
        Self { fields_config }
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
            method,
            path,
            query_string,
            headers = tracing::field::valuable(&headers),
            "http request started"
        );
    }

    #[inline]
    pub fn graphql_request_start(
        &self,
        body_size: usize,
        client_name: Option<&str>,
        client_version: Option<&str>,
        query: &str,
        operation_name: Option<&str>,
        _variables: &HashMap<std::string::String, sonic_rs::Value>,
        _extensions: Option<&HashMap<std::string::String, sonic_rs::Value>>,
    ) {
        let cfg = &self.fields_config.graphql.request;
        let body_size_bytes = cfg.body_size_bytes.then(|| body_size as i64);
        let client_name = cfg.client_name.then(|| client_name);
        let client_version = cfg.client_version.then(|| client_version);
        let operation = cfg.operation.then(|| query);
        let operation_name = cfg.operation_name.then(|| operation_name.unwrap_or(""));
        // let variables = cfg.variables.then(|| variables);
        // let extensions = cfg.extensions.then(|| extensions);

        info!(
            body_size_bytes,
            client_name,
            client_version,
            operation,
            operation_name,
            // TODO: figure this out due to sonic_rs
            // variables = tracing::field::valuable(&variables),
            // extensions = tracing::field::valuable(&extensions),
            "graphql operation started"
        );
    }

    #[inline]
    pub fn graphql_request_end(&self, error_count: usize) {
        let error_count = self
            .fields_config
            .graphql
            .response
            .error_count
            .then(|| error_count);

        info!(error_count, "graphql operation completed");
    }

    #[inline]
    pub fn http_request_end(&self, duration: Duration, response: &ntex::http::Response) {
        let cfg = &self.fields_config.http.response;
        let status_code = cfg.status_code.then(|| response.status().as_u16());
        let duration_ms = cfg.duration_ms.then_some(duration.as_millis());
        let headers =
            (!cfg.headers.is_empty()).then(|| obtain_headers(response.headers(), &cfg.headers));
        let payload_bytes = cfg.payload_bytes.then(|| match response.body().size() {
            BodySize::Empty | BodySize::None => 0,
            BodySize::Sized(size) => size as i64,
            BodySize::Stream => -1,
        });

        info!(
            status_code,
            duration_ms,
            headers = tracing::field::valuable(&headers),
            payload_bytes,
            "http request completed"
        );
    }
}

#[inline]
fn obtain_headers<'req, 'cfg>(
    header_map: &'req ntex::http::HeaderMap,
    headers_list: &'cfg Vec<HttpHeaderName>,
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
