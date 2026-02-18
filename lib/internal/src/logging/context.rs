use hive_router_config::log::service::LogFieldsConfig;
use tracing::info;

pub struct LoggerContext {
    fields_config: LogFieldsConfig,
}

impl LoggerContext {
    pub fn new(fields_config: LogFieldsConfig) -> Self {
        Self { fields_config }
    }

    #[inline]
    pub fn http_request_start(&self, request: &ntex::web::HttpRequest) {
        info!("started processing http request");
    }
}
