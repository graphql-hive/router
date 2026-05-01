use std::borrow::Borrow;
use tracing::{info_span, Span};

use crate::{logging::request_id::RequestIdentifierExtractor, telemetry::otel};

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
    pub fn create(
        extractor: &RequestIdentifierExtractor,
        request: &ntex::web::HttpRequest,
        otel_ctx: &otel::opentelemetry::Context,
    ) -> Self {
        let req_id = extractor.extract_req_id(request);
        let trace_id = extractor.extract_trace_id(otel_ctx);

        let span = info_span!(target: ROUTER_INTERNAL_LOGGER_TARGET, "request",
          req_id = %req_id,
          trace_id = ?trace_id,
        );

        Self { span }
    }
}
