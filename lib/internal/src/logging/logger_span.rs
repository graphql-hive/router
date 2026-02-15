use opentelemetry::TraceId;
use std::borrow::Borrow;
use tracing::{info_span, Span};

use crate::logging::request_id::RequestIdentifier;

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
    pub fn create(req_id: &RequestIdentifier, trace_id: &Option<TraceId>) -> Self {
        let span = info_span!(target: ROUTER_INTERNAL_LOGGER_TARGET, "request",
          req_id = %req_id,
          trace_id = tracing::field::Empty
        );

        if let Some(trace_id) = trace_id {
            span.record("trace_id", trace_id.to_string());
        }

        Self { span }
    }
}
