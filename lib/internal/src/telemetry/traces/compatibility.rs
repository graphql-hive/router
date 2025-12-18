use super::spans::{attributes, kind::HiveSpanKind};
use hive_router_config::telemetry::tracing::SpansSemanticConventionsMode;
use opentelemetry::KeyValue;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use std::collections::HashSet;
use std::fmt::Debug;
use std::str::FromStr;

/// Mapping of spec-compliant to deprecated attribute names for HTTP server spans.
const SERVER_SPEC_TO_DEPRECATED: &[(&str, &str)] = &[
    (
        attributes::HTTP_REQUEST_METHOD,
        attributes::DEPRECATED_HTTP_METHOD,
    ),
    (attributes::URL_FULL, attributes::DEPRECATED_HTTP_URL),
    (attributes::SERVER_ADDRESS, attributes::DEPRECATED_HTTP_HOST),
    (attributes::URL_SCHEME, attributes::DEPRECATED_HTTP_SCHEME),
    (
        attributes::NETWORK_PROTOCOL_VERSION,
        attributes::DEPRECATED_HTTP_FLAVOR,
    ),
    (
        attributes::HTTP_REQUEST_BODY_SIZE,
        attributes::DEPRECATED_HTTP_REQUEST_CONTENT_LENGTH,
    ),
    (
        attributes::USER_AGENT_ORIGINAL,
        attributes::DEPRECATED_HTTP_USER_AGENT,
    ),
    (attributes::URL_PATH, attributes::DEPRECATED_HTTP_TARGET),
    (
        attributes::HTTP_RESPONSE_STATUS_CODE,
        attributes::DEPRECATED_HTTP_STATUS_CODE,
    ),
    (
        attributes::HTTP_RESPONSE_BODY_SIZE,
        attributes::DEPRECATED_HTTP_RESPONSE_CONTENT_LENGTH,
    ),
];

/// Mapping of spec-compliant to deprecated attribute names for HTTP client spans.
const CLIENT_SPEC_TO_DEPRECATED: &[(&str, &str)] = &[
    (
        attributes::HTTP_REQUEST_METHOD,
        attributes::DEPRECATED_HTTP_METHOD,
    ),
    (attributes::URL_FULL, attributes::DEPRECATED_HTTP_URL),
    (
        attributes::SERVER_ADDRESS,
        attributes::DEPRECATED_NET_PEER_NAME,
    ),
    (
        attributes::SERVER_PORT,
        attributes::DEPRECATED_NET_PEER_PORT,
    ),
    (
        attributes::NETWORK_PROTOCOL_VERSION,
        attributes::DEPRECATED_HTTP_FLAVOR,
    ),
    (
        attributes::HTTP_REQUEST_BODY_SIZE,
        attributes::DEPRECATED_HTTP_REQUEST_CONTENT_LENGTH,
    ),
    (
        attributes::HTTP_RESPONSE_STATUS_CODE,
        attributes::DEPRECATED_HTTP_STATUS_CODE,
    ),
    (
        attributes::HTTP_RESPONSE_BODY_SIZE,
        attributes::DEPRECATED_HTTP_RESPONSE_CONTENT_LENGTH,
    ),
];

/// Helper function to find the deprecated key for a given spec-compliant key (server spans).
fn get_server_deprecated_key(spec_key: &str) -> Option<&'static str> {
    SERVER_SPEC_TO_DEPRECATED
        .iter()
        .find(|(k, _)| *k == spec_key)
        .map(|(_, v)| *v)
}

/// Helper function to find the deprecated key for a given spec-compliant key (client spans).
fn get_client_deprecated_key(spec_key: &str) -> Option<&'static str> {
    CLIENT_SPEC_TO_DEPRECATED
        .iter()
        .find(|(k, _)| *k == spec_key)
        .map(|(_, v)| *v)
}

/// An `SpanExporter` that handles HTTP semantic conventions attributes based on the configured mode.
///
/// Depending on the `SpansSemanticConventionsMode`:
/// - `SpecCompliant`- emits only spec-compliant attributes
/// - `Deprecated` - emits only deprecated attributes and removes spec-compliant ones
/// - `SpecAndDeprecated` - emits both spec-compliant and deprecated attributes
///
/// This exporter wrapper inspects spans for HTTP semantic conventions attributes and transforms
/// them based on the configured mode before forwarding to the inner exporter.
///
/// By performing this action in an exporter, the attribute manipulation is moved from the
/// application's hot path to a background thread, improving performance.
#[derive(Debug)]
pub struct HttpCompatibilityExporter<E: SpanExporter> {
    inner: E,
    mode: SpansSemanticConventionsMode,
    server_deprecated_keys: HashSet<&'static str>,
    client_deprecated_keys: HashSet<&'static str>,
}

impl<E: SpanExporter> HttpCompatibilityExporter<E> {
    pub fn new(inner: E, mode: &SpansSemanticConventionsMode) -> Self {
        let mut server_deprecated = HashSet::with_capacity(SERVER_SPEC_TO_DEPRECATED.len());
        for (_spec_key, deprecated_key) in SERVER_SPEC_TO_DEPRECATED {
            server_deprecated.insert(*deprecated_key);
        }

        let mut client_deprecated = HashSet::with_capacity(CLIENT_SPEC_TO_DEPRECATED.len());
        for (_spec_key, deprecated_key) in CLIENT_SPEC_TO_DEPRECATED {
            client_deprecated.insert(*deprecated_key);
        }

        Self {
            inner,
            mode: *mode,
            server_deprecated_keys: server_deprecated,
            client_deprecated_keys: client_deprecated,
        }
    }

    fn process_span_spec_compliant(
        &self,
        span: &mut SpanData,
        deprecated_keys: &HashSet<&'static str>,
    ) {
        span.attributes
            .retain(|attr| !deprecated_keys.contains(attr.key.as_str()));
    }

    fn process_span_deprecated_only(
        &self,
        span: &mut SpanData,
        deprecated_keys: &HashSet<&'static str>,
        get_deprecated_key: fn(&str) -> Option<&'static str>,
    ) {
        let mut deprecated_attrs = Vec::with_capacity(SERVER_SPEC_TO_DEPRECATED.len());
        span.attributes.retain(|attr| {
            if let Some(deprecated_key) = get_deprecated_key(attr.key.as_str()) {
                // This is a spec-compliant attr, so convert it and remove from span
                deprecated_attrs.push(KeyValue::new(deprecated_key, attr.value.clone()));
                false
            } else if deprecated_keys.contains(attr.key.as_str()) {
                // This is already a deprecated attr, so remove it
                false
            } else {
                // Keep everything else
                true
            }
        });

        span.attributes.extend(deprecated_attrs);
    }

    fn process_span_spec_and_deprecated(
        &self,
        span: &mut SpanData,
        get_deprecated_key: fn(&str) -> Option<&'static str>,
    ) {
        let mut deprecated_attrs = Vec::with_capacity(SERVER_SPEC_TO_DEPRECATED.len());

        for attr in span.attributes.iter() {
            if let Some(deprecated_key) = get_deprecated_key(attr.key.as_str()) {
                deprecated_attrs.push(KeyValue::new(deprecated_key, attr.value.clone()));
            }
        }

        if !deprecated_attrs.is_empty() {
            span.attributes.extend(deprecated_attrs);
        }
    }

    fn process_span(&self, span: &mut SpanData) {
        let kind_attr = span
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "hive.kind");

        let Some(kv) = kind_attr else {
            return;
        };

        let opentelemetry::Value::String(kind_attribute_value) = &kv.value else {
            return;
        };

        let Ok(kind) = HiveSpanKind::from_str(kind_attribute_value.as_str()) else {
            return;
        };

        match kind {
            HiveSpanKind::HttpServerRequest => match self.mode {
                SpansSemanticConventionsMode::SpecCompliant => {
                    self.process_span_spec_compliant(span, &self.server_deprecated_keys);
                }
                SpansSemanticConventionsMode::Deprecated => {
                    self.process_span_deprecated_only(
                        span,
                        &self.server_deprecated_keys,
                        get_server_deprecated_key,
                    );
                }
                SpansSemanticConventionsMode::SpecAndDeprecated => {
                    self.process_span_spec_and_deprecated(span, get_server_deprecated_key);
                }
            },
            HiveSpanKind::HttpClientRequest | HiveSpanKind::HttpInflightRequest => {
                match self.mode {
                    SpansSemanticConventionsMode::SpecCompliant => {
                        self.process_span_spec_compliant(span, &self.client_deprecated_keys);
                    }
                    SpansSemanticConventionsMode::Deprecated => {
                        self.process_span_deprecated_only(
                            span,
                            &self.client_deprecated_keys,
                            get_client_deprecated_key,
                        );
                    }
                    SpansSemanticConventionsMode::SpecAndDeprecated => {
                        self.process_span_spec_and_deprecated(span, get_client_deprecated_key);
                    }
                }
            }
            // Other span kinds do not need semantic convention processing.
            _ => {}
        };
    }
}

impl<E: SpanExporter> SpanExporter for HttpCompatibilityExporter<E> {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let processed_batch = batch
            .into_iter()
            .map(|mut span| {
                self.process_span(&mut span);
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
    use crate::telemetry::traces::spans::http_request::{
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

    fn setup_test_pipeline(
        mode: SpansSemanticConventionsMode,
    ) -> (SdkTracerProvider, InMemorySpanExporter) {
        let memory_exporter = InMemorySpanExporterBuilder::new().build();
        let compatibility_exporter = HttpCompatibilityExporter::new(memory_exporter.clone(), &mode);
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

    fn assert_attribute_mapping(
        span: &SpanData,
        deprecated_key: &'static str,
        stable_key: &'static str,
    ) {
        let stable = find_attribute(span, stable_key);
        let deprecated = find_attribute(span, deprecated_key);

        assert_eq!(
            deprecated, stable,
            "Deprecated attribute '{}' and stable attribute '{}' mismatch",
            deprecated_key, stable_key
        );
    }

    #[test]
    fn test_http_server_span_spec_compliant() {
        let (provider, memory_exporter) =
            setup_test_pipeline(SpansSemanticConventionsMode::SpecCompliant);
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
        let http_res =
            ntex::web::HttpResponse::build(ntex::http::StatusCode::OK).body("response body");

        tracer.in_span("root", |_cx| {
            let span = HttpServerRequestSpanBuilder::from_request(&http_req, &body).build();
            span.record_response(&http_res);
        });

        drop(_guard);
        provider.force_flush().unwrap();
        let spans = memory_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2);
        let span = &spans[0];

        // In SpecCompliant mode, only spec-compliant attributes should exist
        assert!(find_attribute(span, "server.address").is_some());
        assert!(find_attribute(span, "http.request.method").is_some());
        assert!(find_attribute(span, "network.protocol.version").is_some());
        assert!(find_attribute(span, "user_agent.original").is_some());
        assert!(find_attribute(span, "http.request.body.size").is_some());
        assert!(find_attribute(span, "url.full").is_some());
        assert!(find_attribute(span, "url.path").is_some());
        assert!(find_attribute(span, "http.response.status_code").is_some());

        // Deprecated attributes should not exist in SpecCompliant mode
        assert!(find_attribute(span, "http.host").is_none());
        assert!(find_attribute(span, "http.method").is_none());
        assert!(find_attribute(span, "http.flavor").is_none());
        assert!(find_attribute(span, "http.user_agent").is_none());
        assert!(find_attribute(span, "http.request_content_length").is_none());
        assert!(find_attribute(span, "http.url").is_none());
        assert!(find_attribute(span, "http.scheme").is_none());
        assert!(find_attribute(span, "http.target").is_none());
        assert!(find_attribute(span, "http.status_code").is_none());
    }

    #[test]
    fn test_http_server_span_deprecated_only() {
        let (provider, memory_exporter) =
            setup_test_pipeline(SpansSemanticConventionsMode::Deprecated);
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
        let http_res =
            ntex::web::HttpResponse::build(ntex::http::StatusCode::OK).body("response body");

        tracer.in_span("root", |_cx| {
            let span = HttpServerRequestSpanBuilder::from_request(&http_req, &body).build();
            span.record_response(&http_res);
        });

        drop(_guard);
        provider.force_flush().unwrap();
        let spans = memory_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2);
        let span = &spans[0];

        // In Deprecated mode, only deprecated attributes should exist
        assert!(find_attribute(span, "http.host").is_some());
        assert!(find_attribute(span, "http.method").is_some());
        assert!(find_attribute(span, "http.flavor").is_some());
        assert!(find_attribute(span, "http.user_agent").is_some());
        assert!(find_attribute(span, "http.request_content_length").is_some());
        assert!(find_attribute(span, "http.url").is_some());
        assert!(find_attribute(span, "http.target").is_some());
        assert!(find_attribute(span, "http.status_code").is_some());

        // Spec-compliant attributes should not exist in Deprecated mode
        assert!(find_attribute(span, "server.address").is_none());
        assert!(find_attribute(span, "http.request.method").is_none());
        assert!(find_attribute(span, "network.protocol.version").is_none());
        assert!(find_attribute(span, "user_agent.original").is_none());
        assert!(find_attribute(span, "http.request.body.size").is_none());
        assert!(find_attribute(span, "url.full").is_none());
        assert!(find_attribute(span, "url.path").is_none());
        assert!(find_attribute(span, "http.response.status_code").is_none());
    }

    #[test]
    fn test_http_server_span_spec_and_deprecated() {
        let (provider, memory_exporter) =
            setup_test_pipeline(SpansSemanticConventionsMode::SpecAndDeprecated);
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
        let http_res =
            ntex::web::HttpResponse::build(ntex::http::StatusCode::OK).body("response body");

        tracer.in_span("root", |_cx| {
            let span = HttpServerRequestSpanBuilder::from_request(&http_req, &body).build();
            span.record_response(&http_res);
        });

        drop(_guard);
        provider.force_flush().unwrap();
        let spans = memory_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2);
        let span = &spans[0];

        // In SpecAndDeprecated mode, both spec-compliant and deprecated attributes should exist
        assert_attribute_mapping(span, "http.host", "server.address");
        assert_attribute_mapping(span, "http.method", "http.request.method");
        assert_attribute_mapping(span, "http.flavor", "network.protocol.version");
        assert_attribute_mapping(span, "http.user_agent", "user_agent.original");
        assert_attribute_mapping(
            span,
            "http.request_content_length",
            "http.request.body.size",
        );
        assert_attribute_mapping(span, "http.url", "url.full");
        assert_attribute_mapping(span, "http.scheme", "url.scheme");
        assert_attribute_mapping(span, "http.target", "url.path");

        // Response attributes are optional since they're only set when record_response() is called
        assert_attribute_mapping(span, "http.status_code", "http.response.status_code");
        assert_attribute_mapping(
            span,
            "http.response_content_length",
            "http.response.body.size",
        );
    }

    #[test]
    fn test_http_client_span_spec_compliant_mode() {
        let (provider, memory_exporter) =
            setup_test_pipeline(SpansSemanticConventionsMode::SpecCompliant);
        let tracer = provider.tracer("test-tracer");
        let _guard = setup_tracing_subscriber(&provider);

        let request = http::Request::builder()
            .method("POST")
            .uri("https://api.example.com:443/v1/users")
            .header(HOST, "api.example.com:443")
            .version(http::Version::HTTP_2)
            .body(Full::from("dummy body"))
            .unwrap();
        let response = http::Response::builder()
            .status(200)
            .body(Full::from("response body"))
            .unwrap();

        tracer.in_span("root", |_cx| {
            let span = HttpClientRequestSpanBuilder::from_request(&request).build();
            span.record_response(&response);
        });

        drop(_guard);
        provider.force_flush().unwrap();
        let spans = memory_exporter.get_finished_spans().unwrap();
        let span = &spans[0];

        // SpecCompliant mode
        // Should have spec-compliant attributes
        assert!(find_attribute(span, "http.request.method").is_some());
        assert!(find_attribute(span, "url.full").is_some());
        assert!(find_attribute(span, "server.address").is_some());
        assert!(find_attribute(span, "server.port").is_some());
        assert!(find_attribute(span, "network.protocol.version").is_some());
        assert!(find_attribute(span, "http.response.status_code").is_some());
        assert!(find_attribute(span, "http.response.body.size").is_some());

        // Should not have deprecated attributes
        assert!(find_attribute(span, "http.method").is_none());
        assert!(find_attribute(span, "http.url").is_none());
        assert!(find_attribute(span, "net.peer.name").is_none());
        assert!(find_attribute(span, "net.peer.port").is_none());
        assert!(find_attribute(span, "http.flavor").is_none());
        assert!(find_attribute(span, "http.status_code").is_none());
        assert!(find_attribute(span, "http.response_content_length").is_none());
    }

    #[test]
    fn test_http_client_span_deprecated_mode() {
        let (provider, memory_exporter) =
            setup_test_pipeline(SpansSemanticConventionsMode::Deprecated);
        let tracer = provider.tracer("test-tracer");
        let _guard = setup_tracing_subscriber(&provider);

        let request = http::Request::builder()
            .method("POST")
            .uri("https://api.example.com:443/v1/users")
            .header(HOST, "api.example.com:443")
            .version(http::Version::HTTP_2)
            .body(Full::from("dummy body"))
            .unwrap();
        let response = http::Response::builder()
            .status(200)
            .body(Full::from("response body"))
            .unwrap();

        tracer.in_span("root", |_cx| {
            let span = HttpClientRequestSpanBuilder::from_request(&request).build();
            span.record_response(&response);
        });

        drop(_guard);
        provider.force_flush().unwrap();
        let spans = memory_exporter.get_finished_spans().unwrap();
        let span = &spans[0];

        // Deprecated mode
        // Should have deprecated attributes
        assert!(find_attribute(span, "http.method").is_some());
        assert!(find_attribute(span, "http.url").is_some());
        assert!(find_attribute(span, "net.peer.name").is_some());
        assert!(find_attribute(span, "net.peer.port").is_some());
        assert!(find_attribute(span, "http.flavor").is_some());
        assert!(find_attribute(span, "http.status_code").is_some());
        assert!(find_attribute(span, "http.response_content_length").is_some());

        // Should not have spec-compliant attributes
        assert!(find_attribute(span, "http.request.method").is_none());
        assert!(find_attribute(span, "url.full").is_none());
        assert!(find_attribute(span, "server.address").is_none());
        assert!(find_attribute(span, "server.port").is_none());
        assert!(find_attribute(span, "network.protocol.version").is_none());
        assert!(find_attribute(span, "http.response.status_code").is_none());
        assert!(find_attribute(span, "http.response.body.size").is_none());
    }

    #[test]
    fn test_http_client_span_spec_and_deprecated_mode() {
        let (provider, memory_exporter) =
            setup_test_pipeline(SpansSemanticConventionsMode::SpecAndDeprecated);
        let tracer = provider.tracer("test-tracer");
        let _guard = setup_tracing_subscriber(&provider);

        let request = http::Request::builder()
            .method("POST")
            .uri("https://api.example.com:443/v1/users")
            .header(HOST, "api.example.com:443")
            .version(http::Version::HTTP_2)
            .body(Full::from("dummy body"))
            .unwrap();
        let response = http::Response::builder()
            .status(200)
            .body(Full::from("response body"))
            .unwrap();

        tracer.in_span("root", |_cx| {
            let span = HttpClientRequestSpanBuilder::from_request(&request).build();
            span.record_response(&response);
        });

        drop(_guard);
        provider.force_flush().unwrap();
        let spans = memory_exporter.get_finished_spans().unwrap();
        let span = &spans[0];

        // SpecAndDeprecated mode
        // Should have both spec-compliant and deprecated attributes
        assert!(find_attribute(span, "http.request.method").is_some());
        assert!(find_attribute(span, "http.method").is_some());
        assert!(find_attribute(span, "url.full").is_some());
        assert!(find_attribute(span, "http.url").is_some());
        assert!(find_attribute(span, "server.address").is_some());
        assert!(find_attribute(span, "net.peer.name").is_some());
        assert!(find_attribute(span, "server.port").is_some());
        assert!(find_attribute(span, "net.peer.port").is_some());
        assert!(find_attribute(span, "network.protocol.version").is_some());
        assert!(find_attribute(span, "http.flavor").is_some());
        assert!(find_attribute(span, "http.response.status_code").is_some());
        assert!(find_attribute(span, "http.status_code").is_some());
        assert!(find_attribute(span, "http.response.body.size").is_some());
        assert!(find_attribute(span, "http.response_content_length").is_some());
        // Verify mappings
        assert_attribute_mapping(span, "net.peer.name", "server.address");
        assert_attribute_mapping(span, "net.peer.port", "server.port");
        assert_attribute_mapping(span, "http.flavor", "network.protocol.version");
        assert_attribute_mapping(span, "http.method", "http.request.method");
        assert_attribute_mapping(span, "http.url", "url.full");
        assert_attribute_mapping(span, "http.status_code", "http.response.status_code");
        assert_attribute_mapping(
            span,
            "http.response_content_length",
            "http.response.body.size",
        );
    }
}
