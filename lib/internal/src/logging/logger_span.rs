use std::{borrow::Borrow, sync::Arc};

use hive_router_config::{log::service::LogFieldsConfig, HiveRouterConfig};
use tracing::{debug_span, field::Empty, Span};

use crate::logging::request_id::obtain_req_correlation_id;

pub static ROUTER_INTERNAL_LOGGER_TARGET: &str = "hive-router-logger";

#[derive(Debug, Clone)]
pub struct LoggerRootSpan {
    pub span: Span,
}

tokio::task_local! {
    pub static LOGGING_ROOT_SPAN_SCOPE: (LoggerRootSpan, Arc<HiveRouterConfig>);
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
        let span = debug_span!(target: ROUTER_INTERNAL_LOGGER_TARGET, "request",
          req_id = %request_id,
        );

        Self { span }
    }
}
