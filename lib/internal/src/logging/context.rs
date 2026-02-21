use std::{collections::BTreeMap, time::Duration};

use hive_router_config::{log::service::LogFieldsConfig, primitives::http_header::HttpHeaderName};
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
        let headers = obtain_headers(request.headers(), &cfg.headers);

        info!(
            method,
            path,
            headers = tracing::field::valuable(&headers),
            "started processing http request"
        );
    }

    #[inline]
    pub fn http_request_end(&self, duration: Duration, response: &ntex::http::Response) {
        let cfg = &self.fields_config.http.response;
        let status_code = cfg.status_code.then(|| response.status().as_u16());
        let duration_ms = cfg.duration_ms.then_some(duration.as_millis());
        let headers = obtain_headers(response.headers(), &cfg.headers);

        info!(
            status_code,
            duration_ms,
            headers = tracing::field::valuable(&headers),
            "http request completed"
        );
    }
}

fn obtain_headers<'req, 'cfg>(
    header_map: &'req ntex::http::HeaderMap,
    headers_list: &'cfg Vec<HttpHeaderName>,
) -> Option<BTreeMap<&'cfg str, Option<&'req str>>> {
    (!headers_list.is_empty()).then(|| {
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
    })
}
