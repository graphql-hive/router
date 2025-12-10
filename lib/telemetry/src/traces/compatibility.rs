use super::spans::kind::HiveSpanKind;
use opentelemetry::KeyValue;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use std::fmt::Debug;
use std::str::FromStr;

/// An `SpanExporter` that adds deprecated HTTP attributes to spans for backward compatibility.
///
/// This exporter wrapper inspects spans for stable HTTP semantic conventions attributes and adds the
/// corresponding deprecated attributes before forwarding them to the inner exporter.
///
/// It correctly handles the differences in attributes between HTTP client and server spans.
///
/// By performing this action in an exporter, the attribute manipulation is moved from the
/// application's hot path to a background thread, improving performance.
#[derive(Debug)]
pub struct HttpCompatibilityExporter<E: SpanExporter> {
    inner: E,
}

impl<E: SpanExporter> HttpCompatibilityExporter<E> {
    /// Creates a new `HttpCompatibilityExporter` that wraps the given exporter.
    pub fn new(inner: E) -> Self {
        Self { inner }
    }
}

/// Adds deprecated attributes for an HTTP Server span.
fn add_server_compat_attributes(span: &mut SpanData) {
    let mut deprecated_attrs = Vec::new();

    for attr in &span.attributes {
        let new_attr = match attr.key.as_str() {
            "http.request.method" => Some(KeyValue::new("http.method", attr.value.clone())),
            "url.full" => Some(KeyValue::new("http.url", attr.value.clone())),
            "server.address" => Some(KeyValue::new("http.host", attr.value.clone())),
            "url.scheme" => Some(KeyValue::new("http.scheme", attr.value.clone())),
            "network.protocol.version" => Some(KeyValue::new("http.flavor", attr.value.clone())),
            "http.request.body.size" => Some(KeyValue::new(
                "http.request_content_length",
                attr.value.clone(),
            )),
            "user_agent.original" => Some(KeyValue::new("http.user_agent", attr.value.clone())),
            "url.path" => Some(KeyValue::new("http.target", attr.value.clone())),
            "http.response.status_code" => {
                Some(KeyValue::new("http.status_code", attr.value.clone()))
            }
            "http.response.body.size" => Some(KeyValue::new(
                "http.response_content_length",
                attr.value.clone(),
            )),
            _ => None,
        };

        if let Some(attr) = new_attr {
            deprecated_attrs.push(attr);
        }
    }

    if !deprecated_attrs.is_empty() {
        span.attributes.extend(deprecated_attrs);
    }
}

/// Adds deprecated attributes for an HTTP Client span.
fn add_client_compat_attributes(span: &mut SpanData) {
    let mut deprecated_attrs = Vec::new();

    for attr in &span.attributes {
        let new_attr = match attr.key.as_str() {
            "http.request.method" => Some(KeyValue::new("http.method", attr.value.clone())),
            "url.full" => Some(KeyValue::new("http.url", attr.value.clone())),
            "server.address" => Some(KeyValue::new("net.peer.name", attr.value.clone())),
            "server.port" => Some(KeyValue::new("net.peer.port", attr.value.clone())),
            "network.protocol.version" => Some(KeyValue::new("http.flavor", attr.value.clone())),
            "http.request.body.size" => Some(KeyValue::new(
                "http.request_content_length",
                attr.value.clone(),
            )),
            "http.response.status_code" => {
                Some(KeyValue::new("http.status_code", attr.value.clone()))
            }
            "http.response.body.size" => Some(KeyValue::new(
                "http.response_content_length",
                attr.value.clone(),
            )),
            _ => None,
        };

        if let Some(attr) = new_attr {
            deprecated_attrs.push(attr);
        }
    }

    if !deprecated_attrs.is_empty() {
        span.attributes.extend(deprecated_attrs);
    }
}

/// Inspects a span and, if it is a known Hive span type, dispatches it to the
/// correct compatibility function.
fn process_span(span: &mut SpanData) {
    let kind_attr = span
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "hive.kind");

    if let Some(kv) = kind_attr {
        if let opentelemetry::Value::String(s) = &kv.value {
            if let Ok(kind) = HiveSpanKind::from_str(s.as_str()) {
                match kind {
                    HiveSpanKind::HttpServerRequest => add_server_compat_attributes(span),
                    HiveSpanKind::HttpClientRequest => add_client_compat_attributes(span),
                    // Other span kinds do not need compatibility attributes.
                    _ => {}
                }
            }
        }
    }
}

impl<E: SpanExporter> SpanExporter for HttpCompatibilityExporter<E> {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let processed_batch = batch
            .into_iter()
            .map(|mut span| {
                process_span(&mut span);
                span
            })
            .collect();

        self.inner.export(processed_batch).await
    }

    fn shutdown(&mut self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn force_flush(&mut self) -> OTelSdkResult {
        self.inner.force_flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::spans::http_request::{
        HttpClientRequestSpanBuilder, HttpServerRequestSpanBuilder,
    };
    use http_body_util::Full;
    use ntex::http::header::{HOST, USER_AGENT};
    use ntex::util::Bytes;
    use opentelemetry::trace::{Tracer, TracerProvider};
    use opentelemetry_sdk::trace::{
        InMemorySpanExporter, InMemorySpanExporterBuilder, SdkTracerProvider, SimpleSpanProcessor,
    };
    use tracing_subscriber::layer::SubscriberExt;

    fn find_attribute<'a>(
        span_data: &'a SpanData,
        key: &'static str,
    ) -> Option<&'a opentelemetry::Value> {
        span_data
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| &kv.value)
    }

    fn setup_test_pipeline() -> (SdkTracerProvider, InMemorySpanExporter) {
        let memory_exporter = InMemorySpanExporterBuilder::new().build();
        let compatibility_exporter = HttpCompatibilityExporter::new(memory_exporter.clone());
        let processor = SimpleSpanProcessor::new(compatibility_exporter);

        let provider = SdkTracerProvider::builder()
            .with_span_processor(processor)
            .build();

        (provider, memory_exporter)
    }

    fn setup_tracing_subscriber(provider: &SdkTracerProvider) -> impl Drop {
        let otel_tracer = provider.tracer("http-tracer");
        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(otel_tracer);
        let subscriber = tracing_subscriber::registry().with(telemetry_layer);
        tracing::subscriber::set_default(subscriber)
    }

    /// Asserts that a deprecated attribute matches the stable attribute.
    fn assert_attribute_mapping(
        span: &SpanData,
        deprecated_key: &'static str,
        stable_key: &'static str,
        message: &str,
    ) {
        assert_eq!(
            find_attribute(span, deprecated_key).unwrap_or_else(|| {
                panic!(
                    "Deprecated attribute '{}' not found in span",
                    deprecated_key
                )
            }),
            find_attribute(span, stable_key).unwrap_or_else(|| {
                panic!("Stable attribute '{}' not found in span", stable_key)
            }),
            "{}",
            message
        );
    }

    #[test]
    fn test_http_server_span_compatibility() {
        let (provider, memory_exporter) = setup_test_pipeline();
        let tracer = provider.tracer("test-tracer");
        let _guard = setup_tracing_subscriber(&provider);

        let req = ntex::web::test::TestRequest::default();
        let req = req
            .uri("/test?q=1")
            .version(ntex::http::Version::HTTP_11)
            .method(ntex::http::Method::GET)
            .header(HOST, "example.com:8080")
            .header(USER_AGENT, "test-agent");
        let body = Bytes::from_static(b"test body");
        let http_req = req.to_http_request();

        tracer.in_span("root", |_cx| {
            let _ = HttpServerRequestSpanBuilder::from_request(&http_req, &body).build();
        });

        drop(_guard);
        provider.force_flush().unwrap();
        let spans = memory_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2);
        let span = &spans[0];

        assert_attribute_mapping(
            span,
            "http.host",
            "server.address",
            "http.host should match server.address",
        );
        assert_attribute_mapping(
            span,
            "http.method",
            "http.request.method",
            "http.method should match http.request.method",
        );
        assert_attribute_mapping(
            span,
            "http.flavor",
            "network.protocol.version",
            "http.flavor should match network.protocol.version",
        );
        assert_attribute_mapping(
            span,
            "http.user_agent",
            "user_agent.original",
            "http.user_agent should match user_agent.original",
        );
        assert_attribute_mapping(
            span,
            "http.request_content_length",
            "http.request.body.size",
            "http.request_content_length should match http.request.body.size",
        );
    }

    #[test]
    fn test_http_client_span_compatibility() {
        let (provider, memory_exporter) = setup_test_pipeline();
        let tracer = provider.tracer("test-tracer");
        let _guard = setup_tracing_subscriber(&provider);

        let request = http::Request::builder()
            .method("POST")
            .uri("https://api.example.com:443/v1/users")
            .header(HOST, "api.example.com:443")
            .version(http::Version::HTTP_2)
            .body(Full::from("dummy body"))
            .unwrap();

        tracer.in_span("root", |_cx| {
            let _ = HttpClientRequestSpanBuilder::from_request(&request).build();
        });

        drop(_guard);
        provider.force_flush().unwrap();
        let spans = memory_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2);
        let span = &spans[0];

        assert_attribute_mapping(
            span,
            "net.peer.name",
            "server.address",
            "net.peer.name should match server.address for client spans",
        );
        assert_attribute_mapping(
            span,
            "net.peer.port",
            "server.port",
            "net.peer.port should match server.port for client spans",
        );
        assert_attribute_mapping(
            span,
            "http.flavor",
            "network.protocol.version",
            "http.flavor should match network.protocol.version",
        );
    }
}
