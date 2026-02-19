use std::time::Duration;

use hive_router_config::log::service::LogFieldsConfig;
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
        let bool_map = &self.fields_config.http.request;
        let method = bool_map.method.then(|| request.method().as_str());
        let path = bool_map.path.then(|| request.path());

        info!(method, path, "started processing http request");
    }

    #[inline]
    pub fn http_request_end(&self, duration: Duration, response: &ntex::http::Response) {
        let bool_map = &self.fields_config.http.response;
        let status_code = bool_map.status_code.then(|| response.status().as_u16());
        let duration_ms = bool_map.duration_ms.then_some(duration.as_millis());

        info!(status_code, duration_ms, "http request completed");
    }
}
