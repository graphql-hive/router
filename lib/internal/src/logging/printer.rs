use tracing::info;

use crate::logging::logger_span::LOGGING_ROOT_SPAN_SCOPE;

pub struct LogPrinter;

impl LogPrinter {
    #[inline]
    pub fn http_request_start(request: &ntex::web::HttpRequest) {
        LOGGING_ROOT_SPAN_SCOPE.with(|(root_span, router_config)| {
            info!("started processing http request");
        })
    }
}
