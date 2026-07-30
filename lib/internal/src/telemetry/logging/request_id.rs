use std::{future::Future, sync::Arc};

use hive_router_config::log::CorrelationConfig;
use http::{HeaderMap, HeaderName};
use ntex::web::HttpRequest;
use opentelemetry::trace::TraceContextExt;
use tokio::task::futures::TaskLocalFuture;
use uuid::Uuid;

use crate::telemetry::otel;

type CorrelationIdentifierKey = &'static str;
type CorrelationIdentifierValue = String;

pub enum RequestIdentifierExtractionPoint<'a> {
    Http(&'a HttpRequest),
    WebSocket(&'a ntex::http::HeaderMap),
}

pub type PluginCorrelationExtractorFn =
    fn(
        &RequestIdentifierExtractionPoint,
    ) -> Option<Vec<(CorrelationIdentifierKey, CorrelationIdentifierValue)>>;

#[derive(Clone)]
pub struct RequestIdentifierExtractor {
    cfg: CorrelationConfig,
    plugin_provided_extractors: Vec<PluginCorrelationExtractorFn>,
}

impl Default for RequestIdentifierExtractor {
    fn default() -> Self {
        Self::new(CorrelationConfig::default())
    }
}

pub struct RequestIdentifiers {
    req_id: String,
    trace_id: Option<String>,
    plugin_provided_correlation_ids:
        Option<Vec<(CorrelationIdentifierKey, CorrelationIdentifierValue)>>,
}

impl RequestIdentifiers {
    pub fn req_id(&self) -> &str {
        self.req_id.as_str()
    }

    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    pub fn plugin_provided_correlation_ids(
        &self,
    ) -> Option<&Vec<(CorrelationIdentifierKey, CorrelationIdentifierValue)>> {
        self.plugin_provided_correlation_ids.as_ref()
    }
}

impl RequestIdentifierExtractor {
    pub fn new(cfg: CorrelationConfig) -> Self {
        Self {
            cfg,
            plugin_provided_extractors: vec![],
        }
    }

    pub fn extract(
        &self,
        source: RequestIdentifierExtractionPoint,
        otel_ctx: &otel::opentelemetry::Context,
    ) -> RequestIdentifiers {
        let req_id = match source {
            RequestIdentifierExtractionPoint::Http(req) => self.extract_req_id(req),
            RequestIdentifierExtractionPoint::WebSocket(h) => self.extract_req_id(h),
        };
        let trace_id = self.extract_trace_id(otel_ctx);
        let plugin_provided_correlation_ids = self.extract_plugin_provided_correlation_ids(source);

        RequestIdentifiers {
            req_id,
            trace_id: trace_id.map(|id| id.to_string()),
            plugin_provided_correlation_ids,
        }
    }

    fn extract_plugin_provided_correlation_ids(
        &self,
        source: RequestIdentifierExtractionPoint,
    ) -> Option<Vec<(CorrelationIdentifierKey, CorrelationIdentifierValue)>> {
        if self.plugin_provided_extractors.is_empty() {
            return None;
        }
        let mut correlation_ids = Vec::new();

        for extractor in &self.plugin_provided_extractors {
            if let Some(mut extracted) = extractor(&source) {
                correlation_ids.append(&mut extracted);
            }
        }

        Some(correlation_ids)
    }

    fn extract_trace_id(
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

    fn sanitize_request_id(raw: &str) -> Option<&str> {
        const MAX_LEN: usize = 128;

        if raw.is_empty() || raw.len() > MAX_LEN {
            return None;
        }

        let valid = raw.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+' | b'/' | b'=')
        });

        valid.then_some(raw)
    }

    fn extract_req_id(&self, headers: &impl HeaderLookup) -> String {
        if let Some(req_id_header) = headers
            .lookup_str(self.cfg.id_header.get_header_ref())
            .and_then(Self::sanitize_request_id)
        {
            return req_id_header.to_string();
        }

        Uuid::now_v7().to_string()
    }
}

/// Abstracts a header-name → `&str` lookup so `extract_req_id` works over both
/// the `http` and `ntex` header types without duplication.
pub trait HeaderLookup {
    fn lookup_str(&self, name: &HeaderName) -> Option<&str>;
}

impl HeaderLookup for HeaderMap {
    fn lookup_str(&self, name: &HeaderName) -> Option<&str> {
        self.get(name).and_then(|v| v.to_str().ok())
    }
}

impl HeaderLookup for ntex::http::HeaderMap {
    fn lookup_str(&self, name: &HeaderName) -> Option<&str> {
        self.get(name.as_str()).and_then(|v| v.to_str().ok())
    }
}

impl HeaderLookup for HttpRequest {
    fn lookup_str(&self, name: &HeaderName) -> Option<&str> {
        self.headers().lookup_str(name)
    }
}

impl HeaderLookup for &HttpRequest {
    fn lookup_str(&self, name: &HeaderName) -> Option<&str> {
        self.headers().lookup_str(name)
    }
}

tokio::task_local! {
    pub static REQUEST_IDENTIFIERS: Arc<RequestIdentifiers>;
}

pub trait WithRequestIdentifiers: Future + Sized {
    fn with_request_id(
        self,
        identifiers: Arc<RequestIdentifiers>,
    ) -> TaskLocalFuture<Arc<RequestIdentifiers>, Self> {
        REQUEST_IDENTIFIERS.scope(identifiers, self)
    }
}

impl<F: Future> WithRequestIdentifiers for F {}
