use std::{
    fmt::Display,
    time::{SystemTime, UNIX_EPOCH},
};

use hive_router_config::log::service::CorrelationConfig;
use ntex::web::HttpRequest;
use opentelemetry::trace::TraceContextExt;
use sonyflake::Sonyflake;
use tracing::warn;

use crate::telemetry::otel;

#[derive(Clone)]
pub struct RequestIdentifierExtractor {
    generator: Sonyflake,
    cfg: CorrelationConfig,
}

impl Default for RequestIdentifierExtractor {
    fn default() -> Self {
        Self::new(CorrelationConfig::default())
    }
}

impl RequestIdentifierExtractor {
    pub fn new(cfg: CorrelationConfig) -> Self {
        Self {
            generator: Sonyflake::new().expect("Failed to create Sonyflake"),
            cfg,
        }
    }

    pub fn extract_trace_id(
        &self,
        otel_ctx: &otel::opentelemetry::Context,
    ) -> Option<opentelemetry::trace::TraceId> {
        if !self.cfg.trace_propagation {
            return None;
        }

        let span_ref = otel_ctx.span();
        let context_ref = span_ref.span_context();

        if context_ref.is_valid() {
            return Some(context_ref.trace_id());
        }

        None
    }

    pub fn extract_req_id<'req>(&self, request: &'req HttpRequest) -> RequestIdentifier<'req> {
        if let Some(req_id_header) = request
            .headers()
            .get(self.cfg.id_header.get_header_ref())
            .and_then(|v| v.to_str().ok())
        {
            return RequestIdentifier::FromRequest(req_id_header);
        }

        match self.generator.next_id() {
            Ok(id) => {
                return RequestIdentifier::Generated(id);
            }
            Err(e) => {
                warn!(error = %e, "Failed to generate local request id, will fallback to timestamp");
            }
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards, please confirm your system/os clock")
            .as_secs();

        RequestIdentifier::Generated(timestamp)
    }
}

pub enum RequestIdentifier<'a> {
    FromRequest(&'a str),
    Generated(u64),
}

impl Display for RequestIdentifier<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestIdentifier::FromRequest(id) => write!(f, "{}", id),
            RequestIdentifier::Generated(id) => write!(f, "{}", id),
        }
    }
}
