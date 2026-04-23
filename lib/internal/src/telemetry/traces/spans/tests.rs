use super::attributes;
use super::graphql::{
    GraphQLAuthorizeSpan, GraphQLExecuteSpan, GraphQLNormalizeSpan, GraphQLOperationSpan,
    GraphQLParseSpan, GraphQLPlanSpan, GraphQLSpanOperationIdentity, GraphQLSubgraphOperationSpan,
    GraphQLValidateSpan, GraphQLVariableCoercionSpan,
};
use super::http_request::{HttpClientRequestSpan, HttpInflightRequestSpan, HttpServerRequestSpan};
use crate::graphql::ObservedError;
use crate::telemetry::traces::spans::http_request::HttpServerSpanRequest;
use bytes::Bytes;
use hive_router_config::telemetry::{ClientIpHeaderConfig, ClientIpHeaderTrustedProxiesConfig};
use http::header::{FORWARDED, HOST, USER_AGENT};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, Version};
use http_body_util::Full;
use ntex::http::HeaderMap as NtexHeaderMap;
use ntex::util::Bytes as NtexBytes;
use ntex::web::test::TestRequest;
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::Span;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::Registry;

const XFF: HeaderName = HeaderName::from_static("x-forwarded-for");

struct HttpRequestMock {
    req: ntex::web::HttpRequest,
    // peer_addr is stored separately as TestRequest never passes it to HttpRequest.
    // It's gone and we need it to test trusted_proxies thingy.
    peer_addr: Option<SocketAddr>,
}

impl HttpRequestMock {
    fn with_peer_addr(mut self, peer_addr: SocketAddr) -> Self {
        self.peer_addr = Some(peer_addr);
        self
    }
}

impl From<TestRequest> for HttpRequestMock {
    fn from(req: TestRequest) -> Self {
        Self {
            req: req.to_http_request(),
            peer_addr: None,
        }
    }
}

impl HttpServerSpanRequest for HttpRequestMock {
    fn headers(&self) -> &NtexHeaderMap {
        self.req.headers()
    }

    fn method(&self) -> &Method {
        self.req.method()
    }

    fn uri(&self) -> &Uri {
        self.req.uri()
    }

    fn version(&self) -> Version {
        self.req.version()
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr
    }
}

#[derive(Clone, Default)]
struct RecordingLayer {
    fields: Arc<Mutex<HashMap<u64, BTreeMap<String, String>>>>,
}

impl RecordingLayer {
    fn value(&self, id: u64, key: &str) -> Option<String> {
        self.fields
            .lock()
            .expect("recording layer lock")
            .get(&id)
            .and_then(|fields| fields.get(key).cloned())
    }

    fn assert_recorded_value(&self, span: &Span, key: &str, expected: &str) {
        let id = span.id().expect("span id").into_u64();
        assert_eq!(self.value(id, key).as_deref(), Some(expected));
    }

    fn assert_not_recorded(&self, span: &Span, key: &str) {
        let id = span.id().expect("span id").into_u64();
        assert!(
            self.value(id, key).is_none(),
            "Expected '{key}' not to be recorded"
        );
    }
}

#[derive(Default)]
struct FieldVisitor {
    fields: BTreeMap<String, String>,
}

impl FieldVisitor {
    fn insert(&mut self, field: &Field, value: String) {
        self.fields.insert(field.name().to_string(), value);
    }
}

impl Visit for FieldVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, value.to_string());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.insert(field, format!("{value:?}"));
    }
}

impl<S> Layer<S> for RecordingLayer
where
    S: tracing::Subscriber,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        _ctx: Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor::default();
        attrs.record(&mut visitor);
        let mut fields = self.fields.lock().expect("recording layer lock");
        fields.insert(id.clone().into_u64(), visitor.fields);
    }

    fn on_record(
        &self,
        id: &tracing::Id,
        values: &tracing::span::Record<'_>,
        _ctx: Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor::default();
        values.record(&mut visitor);
        let mut fields = self.fields.lock().expect("recording layer lock");
        let entry = fields.entry(id.clone().into_u64()).or_default();
        entry.extend(visitor.fields);
    }
}

fn assert_fields(span: &Span, expected_fields: &[&str]) {
    let metadata = span.metadata().expect("Span should have metadata");

    // Look for missing fields
    for field in expected_fields {
        assert!(
            span.field(*field).is_some(),
            "Field '{}' is missing from span '{}'",
            field,
            metadata.name()
        );
    }

    // Look for extra fields
    let extra_fields = metadata
        .fields()
        .iter()
        .map(|field| field.name())
        .filter(|f| !expected_fields.contains(f))
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        expected_fields.len(),
        metadata.fields().len(),
        "Found extra fields in the span: {}",
        extra_fields
    );
}

fn ip_header_config(header: &str) -> Option<ClientIpHeaderConfig> {
    Some(ClientIpHeaderConfig::HeaderName(header.into()))
}

fn trusted_ip_header_config(
    header: &str,
    trusted_proxies: Vec<&str>,
) -> Option<ClientIpHeaderConfig> {
    Some(ClientIpHeaderConfig::TrustedProxies(
        ClientIpHeaderTrustedProxiesConfig {
            name: header.into(),
            trusted_proxies: trusted_proxies.into_iter().map(Into::into).collect(),
        },
    ))
}

#[test]
fn test_http_server_request_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());
    let response_body = "response body";

    with_default(subscriber, || {
        let req = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(USER_AGENT, "test-agent")
            .header(XFF, "192.168.0.1, 192.168.0.2, 192.168.0.3:420")
            .to_http_request();
        let body = NtexBytes::from("test body");

        let span = HttpServerRequestSpan::from_request(&req, &ip_header_config(XFF.as_str()));
        span.record_body_size(body.len());
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::SERVER_ADDRESS,
                attributes::SERVER_PORT,
                attributes::URL_FULL,
                attributes::URL_PATH,
                attributes::URL_SCHEME,
                attributes::HTTP_REQUEST_BODY_SIZE,
                attributes::HTTP_REQUEST_METHOD,
                attributes::NETWORK_PROTOCOL_VERSION,
                attributes::USER_AGENT_ORIGINAL,
                attributes::HTTP_RESPONSE_STATUS_CODE,
                attributes::HTTP_RESPONSE_BODY_SIZE,
                attributes::HTTP_ROUTE,
                attributes::CLIENT_ADDRESS,
                attributes::CLIENT_PORT,
                attributes::NETWORK_PEER_ADDRESS,
                attributes::NETWORK_PEER_PORT,
            ],
        );

        let response = ntex::web::HttpResponse::Ok().body(response_body);
        span.record_response(&response);

        layer.assert_recorded_value(&span, attributes::HTTP_RESPONSE_STATUS_CODE, "200");
        layer.assert_recorded_value(
            &span,
            attributes::HTTP_RESPONSE_BODY_SIZE,
            &response_body.len().to_string(),
        );
        layer.assert_recorded_value(&span, attributes::OTEL_STATUS_CODE, "Ok");
        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "192.168.0.1");
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);

        let span = HttpServerRequestSpan::from_request(&req, &ip_header_config(XFF.as_str()));
        span.record_internal_server_error();

        layer.assert_recorded_value(&span, attributes::HTTP_RESPONSE_STATUS_CODE, "500");
        layer.assert_recorded_value(&span, attributes::OTEL_STATUS_CODE, "Error");
        layer.assert_recorded_value(&span, attributes::ERROR_TYPE, "500");
    });
}

#[test]
fn test_http_server_request_span_client_address_from_various_values() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());

    with_default(subscriber, || {
        let realistic_with_port = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(XFF, "10.0.0.1:1234, 172.16.1.42:443")
            .to_http_request();
        let span = HttpServerRequestSpan::from_request(
            &realistic_with_port,
            &ip_header_config(XFF.as_str()),
        );
        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "10.0.0.1");
        layer.assert_recorded_value(&span, attributes::CLIENT_PORT, "1234");

        let realistic_without_port = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(XFF, "10.0.0.1, 172.16.1.42")
            .to_http_request();
        let span = HttpServerRequestSpan::from_request(
            &realistic_without_port,
            &ip_header_config(XFF.as_str()),
        );
        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "10.0.0.1");

        let realistic_ipv6_with_port = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(XFF, "[2001:db8::1]:8080, [2001:db8::2]:8443")
            .to_http_request();
        let span = HttpServerRequestSpan::from_request(
            &realistic_ipv6_with_port,
            &ip_header_config(XFF.as_str()),
        );
        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "2001:db8::1");
        layer.assert_recorded_value(&span, attributes::CLIENT_PORT, "8080");

        let realistic_ipv6_without_port = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(XFF, "2001:db8::1, 2001:db8::2")
            .to_http_request();
        let span = HttpServerRequestSpan::from_request(
            &realistic_ipv6_without_port,
            &ip_header_config(XFF.as_str()),
        );
        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "2001:db8::1");
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);

        let unrealistic_malformed_ipv6_with_port = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(XFF, "2001:db8::2:8443")
            .to_http_request();
        let span = HttpServerRequestSpan::from_request(
            &unrealistic_malformed_ipv6_with_port,
            &ip_header_config(XFF.as_str()),
        );
        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "2001:db8::2:8443");
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);

        let unrealistic_empty = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(XFF, ", ,")
            .to_http_request();
        let span = HttpServerRequestSpan::from_request(
            &unrealistic_empty,
            &ip_header_config(XFF.as_str()),
        );
        layer.assert_not_recorded(&span, attributes::CLIENT_ADDRESS);
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);
    });
}

#[test]
fn test_http_server_request_span_client_address_with_trusted_proxies() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());
    let peer_addr = SocketAddr::from_str("10.0.0.2:8080").unwrap();
    let peer_ip = peer_addr.ip().to_string();
    let peer_port = peer_addr.port().to_string();

    with_default(subscriber, || {
        let req = HttpRequestMock::from(
            TestRequest::with_uri("/graphql")
                .header(HOST, "localhost:8080")
                .header(XFF, "198.51.100.7, 10.0.0.2"),
        )
        .with_peer_addr(peer_addr);

        let span = HttpServerRequestSpan::from_request(
            &req,
            &trusted_ip_header_config(XFF.as_str(), vec!["10.0.0.0/8"]),
        );

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "198.51.100.7");
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);

        let all_trusted = HttpRequestMock::from(
            TestRequest::with_uri("/graphql")
                .header(HOST, "localhost:8080")
                .header(XFF, "10.1.1.1, 10.2.2.2"),
        )
        .with_peer_addr(peer_addr);

        let span = HttpServerRequestSpan::from_request(
            &all_trusted,
            &trusted_ip_header_config(XFF.as_str(), vec!["10.0.0.0/8"]),
        );

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "10.1.1.1");
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);

        let mixed_invalid = HttpRequestMock::from(
            TestRequest::with_uri("/graphql")
                .header(HOST, "localhost:8080")
                .header(XFF, "garbage, 198.51.100.7, 10.0.0.2"),
        )
        .with_peer_addr(peer_addr);

        let span = HttpServerRequestSpan::from_request(
            &mixed_invalid,
            &trusted_ip_header_config(XFF.as_str(), vec!["10.0.0.0/8"]),
        );

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "198.51.100.7");
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);

        let non_ip_tokens = HttpRequestMock::from(
            TestRequest::with_uri("/graphql")
                .header(HOST, "localhost:8080")
                .header(XFF, "foo:300, local"),
        )
        .with_peer_addr(peer_addr);

        let span = HttpServerRequestSpan::from_request(
            &non_ip_tokens,
            &trusted_ip_header_config(XFF.as_str(), vec!["10.0.0.0/8"]),
        );

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, peer_ip.as_str());
        layer.assert_recorded_value(&span, attributes::CLIENT_PORT, peer_port.as_str());
    });
}

#[test]
fn test_http_server_request_span_client_address_from_forwarded() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());

    with_default(subscriber, || {
        let req = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(FORWARDED, "for=198.51.100.7;proto=https, for=10.0.0.2")
            .to_http_request();

        let span = HttpServerRequestSpan::from_request(&req, &ip_header_config(FORWARDED.as_str()));

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "198.51.100.7");
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);

        let req = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(FORWARDED, r#"for="198.51.100.7:1234";proto=https"#)
            .to_http_request();

        let span = HttpServerRequestSpan::from_request(&req, &ip_header_config(FORWARDED.as_str()));

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "198.51.100.7");
        layer.assert_recorded_value(&span, attributes::CLIENT_PORT, "1234");

        let req = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(FORWARDED, r#"for="[2001:db8::1]:8080";proto=https"#)
            .to_http_request();

        let span = HttpServerRequestSpan::from_request(&req, &ip_header_config(FORWARDED.as_str()));

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "2001:db8::1");
        layer.assert_recorded_value(&span, attributes::CLIENT_PORT, "8080");

        let req = TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(FORWARDED, "for=_hidden, proto=https")
            .to_http_request();

        let span = HttpServerRequestSpan::from_request(&req, &ip_header_config(FORWARDED.as_str()));

        layer.assert_not_recorded(&span, attributes::CLIENT_ADDRESS);
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);
    });
}

#[test]
fn test_http_server_request_span_client_address_from_forwarded_trusted_proxies() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());
    let peer_addr = SocketAddr::from_str("10.0.0.2:8080").unwrap();
    let peer_ip = peer_addr.ip().to_string();

    with_default(subscriber, || {
        let req = HttpRequestMock::from(
            TestRequest::with_uri("/graphql")
                .header(HOST, "localhost:8080")
                .header(FORWARDED, "for=198.51.100.7;proto=https, for=10.0.0.2"),
        )
        .with_peer_addr(peer_addr);

        let span = HttpServerRequestSpan::from_request(
            &req,
            &trusted_ip_header_config(FORWARDED.as_str(), vec!["10.0.0.0/8"]),
        );

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "198.51.100.7");
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);

        let all_trusted = HttpRequestMock::from(
            TestRequest::with_uri("/graphql")
                .header(HOST, "localhost:8080")
                .header(FORWARDED, "for=10.1.1.1, for=10.2.2.2"),
        )
        .with_peer_addr(peer_addr);

        let span = HttpServerRequestSpan::from_request(
            &all_trusted,
            &trusted_ip_header_config(FORWARDED.as_str(), vec!["10.0.0.0/8"]),
        );

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "10.1.1.1");
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);

        let with_port = HttpRequestMock::from(
            TestRequest::with_uri("/graphql")
                .header(HOST, "localhost:8080")
                .header(FORWARDED, r#"for="198.51.100.7:4444", for=10.0.0.2"#),
        )
        .with_peer_addr(peer_addr);

        let span = HttpServerRequestSpan::from_request(
            &with_port,
            &trusted_ip_header_config(FORWARDED.as_str(), vec!["10.0.0.0/8"]),
        );

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, "198.51.100.7");
        layer.assert_recorded_value(&span, attributes::CLIENT_PORT, "4444");

        let invalid_tokens = HttpRequestMock::from(
            TestRequest::with_uri("/graphql")
                .header(HOST, "localhost:8080")
                .header(FORWARDED, "for=_hidden, for=10.0.0.2"),
        )
        .with_peer_addr(peer_addr);

        let span = HttpServerRequestSpan::from_request(
            &invalid_tokens,
            &trusted_ip_header_config(FORWARDED.as_str(), vec!["10.0.0.0/8"]),
        );

        layer.assert_recorded_value(&span, attributes::CLIENT_ADDRESS, peer_ip.as_str());
        layer.assert_not_recorded(&span, attributes::CLIENT_PORT);
    });
}

#[test]
fn test_http_client_request_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());
    let response_body = Bytes::from("client response");

    with_default(subscriber, || {
        let req = http::Request::builder()
            .uri("http://localhost:8081/graphql")
            .header(USER_AGENT, "test-agent")
            .body(Full::new(Bytes::from("test body")))
            .unwrap();

        let span = HttpClientRequestSpan::from_request(&req);
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::SERVER_ADDRESS,
                attributes::SERVER_PORT,
                attributes::URL_FULL,
                attributes::URL_PATH,
                attributes::URL_SCHEME,
                attributes::HTTP_REQUEST_BODY_SIZE,
                attributes::HTTP_REQUEST_METHOD,
                attributes::NETWORK_PROTOCOL_VERSION,
                attributes::USER_AGENT_ORIGINAL,
                attributes::HTTP_RESPONSE_STATUS_CODE,
                attributes::HTTP_RESPONSE_BODY_SIZE,
            ],
        );

        let response = http::Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(response_body.clone()))
            .unwrap();
        span.record_response(&response);

        layer.assert_recorded_value(&span, attributes::HTTP_RESPONSE_STATUS_CODE, "200");
        layer.assert_recorded_value(
            &span,
            attributes::HTTP_RESPONSE_BODY_SIZE,
            &response_body.len().to_string(),
        );
        layer.assert_recorded_value(&span, attributes::OTEL_STATUS_CODE, "Ok");

        let req = http::Request::builder()
            .uri("http://localhost:8081/graphql")
            .body(Full::new(Bytes::from("test body")))
            .unwrap();
        let span = HttpClientRequestSpan::from_request(&req);
        span.record_internal_server_error();

        layer.assert_recorded_value(&span, attributes::HTTP_RESPONSE_STATUS_CODE, "500");
        layer.assert_recorded_value(&span, attributes::OTEL_STATUS_CODE, "Error");
        layer.assert_recorded_value(&span, attributes::ERROR_TYPE, "500");
    });
}

#[test]
fn test_http_inflight_request_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());
    let response_body = Bytes::from("inflight response");

    with_default(subscriber, || {
        let method = Method::POST;
        let url = Uri::from_static("http://localhost:8082/graphql");
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("localhost:8082"));
        headers.insert(USER_AGENT, HeaderValue::from_static("test-agent"));
        let body = b"test body";

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body);
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::HIVE_INFLIGHT_ROLE,
                attributes::HIVE_INFLIGHT_KEY,
                attributes::SERVER_ADDRESS,
                attributes::SERVER_PORT,
                attributes::URL_FULL,
                attributes::URL_PATH,
                attributes::URL_SCHEME,
                attributes::HTTP_REQUEST_BODY_SIZE,
                attributes::HTTP_REQUEST_METHOD,
                attributes::NETWORK_PROTOCOL_VERSION,
                attributes::USER_AGENT_ORIGINAL,
                attributes::HTTP_RESPONSE_STATUS_CODE,
                attributes::HTTP_RESPONSE_BODY_SIZE,
            ],
        );

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body);
        span.record_as_leader(&1);
        layer.assert_recorded_value(&span, attributes::HIVE_INFLIGHT_ROLE, "leader");
        layer.assert_recorded_value(&span, attributes::HIVE_INFLIGHT_KEY, &1.to_string());

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body);
        span.record_as_joiner(&2);
        layer.assert_recorded_value(&span, attributes::HIVE_INFLIGHT_ROLE, "joiner");
        layer.assert_recorded_value(&span, attributes::HIVE_INFLIGHT_KEY, &2.to_string());

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body);
        span.record_response(&response_body, &StatusCode::OK);

        layer.assert_recorded_value(&span, attributes::HTTP_RESPONSE_STATUS_CODE, "200");
        layer.assert_recorded_value(
            &span,
            attributes::HTTP_RESPONSE_BODY_SIZE,
            &response_body.len().to_string(),
        );
        layer.assert_recorded_value(&span, attributes::OTEL_STATUS_CODE, "Ok");

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body);
        span.record_internal_server_error();

        layer.assert_recorded_value(&span, attributes::HTTP_RESPONSE_STATUS_CODE, "500");
        layer.assert_recorded_value(&span, attributes::OTEL_STATUS_CODE, "Error");
        layer.assert_recorded_value(&span, attributes::ERROR_TYPE, "500");
    });
}

#[test]
fn test_graphql_parse_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());

    with_default(subscriber, || {
        let span = GraphQLParseSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_KIND,
                attributes::CACHE_HIT,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_DOCUMENT_HASH,
            ],
        );

        span.record_cache_hit(true);
        layer.assert_recorded_value(&span, attributes::CACHE_HIT, "true");

        let identity = GraphQLSpanOperationIdentity {
            name: Some("GetMe"),
            operation_type: "query",
            client_document_hash: "hash123",
        };
        span.record_operation_identity(identity);
        layer.assert_recorded_value(&span, attributes::GRAPHQL_OPERATION_NAME, "GetMe");
        layer.assert_recorded_value(&span, attributes::GRAPHQL_OPERATION_TYPE, "query");
        layer.assert_recorded_value(&span, attributes::GRAPHQL_DOCUMENT_HASH, "hash123");
    });
}

#[test]
fn test_graphql_validate_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());

    with_default(subscriber, || {
        let span = GraphQLValidateSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_KIND,
                attributes::CACHE_HIT,
            ],
        );

        span.record_cache_hit(false);
        layer.assert_recorded_value(&span, attributes::CACHE_HIT, "false");
    });
}

#[test]
fn test_graphql_variable_coercion_span() {
    let span = GraphQLVariableCoercionSpan::new();
    assert_fields(&span, &[attributes::HIVE_KIND, attributes::OTEL_KIND]);
}

#[test]
fn test_graphql_normalize_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());

    with_default(subscriber, || {
        let span = GraphQLNormalizeSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_KIND,
                attributes::CACHE_HIT,
            ],
        );

        span.record_cache_hit(true);
        layer.assert_recorded_value(&span, attributes::CACHE_HIT, "true");
    });
}

#[test]
fn test_graphql_authorize_span() {
    let span = GraphQLAuthorizeSpan::new();
    assert_fields(&span, &[attributes::HIVE_KIND, attributes::OTEL_KIND]);
}

#[test]
fn test_graphql_plan_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());

    with_default(subscriber, || {
        let span = GraphQLPlanSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_KIND,
                attributes::CACHE_HIT,
            ],
        );

        span.record_cache_hit(false);
        layer.assert_recorded_value(&span, attributes::CACHE_HIT, "false");
    });
}

#[test]
fn test_graphql_execute_span() {
    let span = GraphQLExecuteSpan::new();
    assert_fields(&span, &[attributes::HIVE_KIND, attributes::OTEL_KIND]);
}

#[test]
fn test_graphql_operation_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());

    with_default(subscriber, || {
        let span = GraphQLOperationSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_OPERATION_ID,
                attributes::GRAPHQL_DOCUMENT_HASH,
                attributes::GRAPHQL_DOCUMENT,
                attributes::HIVE_GRAPHQL_ERROR_COUNT,
                attributes::HIVE_GRAPHQL_ERROR_CODES,
                attributes::HIVE_CLIENT_NAME,
                attributes::HIVE_CLIENT_VERSION,
                attributes::HIVE_GRAPHQL_OPERATION_HASH,
            ],
        );

        span.record_error_count(3);
        layer.assert_recorded_value(&span, attributes::HIVE_GRAPHQL_ERROR_COUNT, "3");

        span.record_errors(|| {
            vec![
                ObservedError {
                    code: Some("BETA".to_string()),
                    message: "error one".to_string(),
                    path: None,
                    service_name: None,
                    affected_path: None,
                },
                ObservedError {
                    code: Some("ALPHA".to_string()),
                    message: "error two".to_string(),
                    path: None,
                    service_name: None,
                    affected_path: None,
                },
                ObservedError {
                    code: Some("BETA".to_string()),
                    message: "error three".to_string(),
                    path: None,
                    service_name: None,
                    affected_path: None,
                },
            ]
        });
        layer.assert_recorded_value(&span, attributes::HIVE_GRAPHQL_ERROR_CODES, "ALPHA,BETA");

        let identity = GraphQLSpanOperationIdentity {
            name: Some("GetMe"),
            operation_type: "query",
            client_document_hash: "doc-hash",
        };
        span.record_details(
            "query GetMe { me }",
            identity,
            Some("client"),
            Some("1.0.0"),
            "op-hash",
        );
        layer.assert_recorded_value(&span, attributes::GRAPHQL_DOCUMENT, "query GetMe { me }");
        layer.assert_recorded_value(&span, attributes::GRAPHQL_OPERATION_NAME, "GetMe");
        layer.assert_recorded_value(&span, attributes::GRAPHQL_OPERATION_TYPE, "query");
        layer.assert_recorded_value(&span, attributes::GRAPHQL_DOCUMENT_HASH, "doc-hash");
        layer.assert_recorded_value(&span, attributes::HIVE_GRAPHQL_OPERATION_HASH, "op-hash");
        layer.assert_recorded_value(&span, attributes::HIVE_CLIENT_NAME, "client");
        layer.assert_recorded_value(&span, attributes::HIVE_CLIENT_VERSION, "1.0.0");
    });
}

#[test]
fn test_graphql_subgraph_operation_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());

    with_default(subscriber, || {
        let span = GraphQLSubgraphOperationSpan::new("test-subgraph", "query Example { me }");
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_DOCUMENT_HASH,
                attributes::GRAPHQL_DOCUMENT,
                attributes::HIVE_GRAPHQL_ERROR_COUNT,
                attributes::HIVE_GRAPHQL_ERROR_CODES,
                attributes::HIVE_GRAPHQL_SUBGRAPH_NAME,
            ],
        );

        span.record_error_count(2);
        layer.assert_recorded_value(&span, attributes::HIVE_GRAPHQL_ERROR_COUNT, "2");

        span.record_errors(|| {
            vec![
                ObservedError {
                    code: Some("UNAUTHENTICATED".to_string()),
                    message: "error two".to_string(),
                    path: None,
                    service_name: None,
                    affected_path: None,
                },
                ObservedError {
                    code: Some("BAD_USER_INPUT".to_string()),
                    message: "error one".to_string(),
                    path: None,
                    service_name: None,
                    affected_path: None,
                },
            ]
        });
        layer.assert_recorded_value(
            &span,
            attributes::HIVE_GRAPHQL_ERROR_CODES,
            "BAD_USER_INPUT,UNAUTHENTICATED",
        );

        let identity = GraphQLSpanOperationIdentity {
            name: Some("GetMe"),
            operation_type: "query",
            client_document_hash: "hash123",
        };
        span.record_operation_identity(identity);
        layer.assert_recorded_value(&span, attributes::GRAPHQL_OPERATION_NAME, "GetMe");
        layer.assert_recorded_value(&span, attributes::GRAPHQL_OPERATION_TYPE, "query");
        layer.assert_recorded_value(&span, attributes::GRAPHQL_DOCUMENT_HASH, "hash123");
    });
}
