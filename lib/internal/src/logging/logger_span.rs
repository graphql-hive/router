use std::borrow::Borrow;

use tracing::{info_span, Span};

use crate::logging::request_id::obtain_req_correlation_id;

pub static ROUTER_INTERNAL_LOGGER_TARGET: &str = "hive-router-logger";

#[derive(Debug, Clone)]
pub struct LoggerRootSpan {
    pub span: Span,
}

impl std::ops::Deref for LoggerRootSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Borrow<Span> for LoggerRootSpan {
    fn borrow(&self) -> &Span {
        &self.span
    }
}

impl LoggerRootSpan {
    pub fn create(request: &ntex::web::HttpRequest) -> Self {
        let request_id = obtain_req_correlation_id(request);
        let span = info_span!(target: ROUTER_INTERNAL_LOGGER_TARGET, "request",
          req_id = %request_id,
        );

        Self { span }
    }
}
