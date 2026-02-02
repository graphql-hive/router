use super::attributes;
use super::graphql::{
    GraphQLAuthorizeSpan, GraphQLExecuteSpan, GraphQLNormalizeSpan, GraphQLOperationSpan,
    GraphQLParseSpan, GraphQLPlanSpan, GraphQLSpanOperationIdentity, GraphQLSubgraphOperationSpan,
    GraphQLValidateSpan, GraphQLVariableCoercionSpan,
};
use super::http_request::{HttpClientRequestSpan, HttpInflightRequestSpan, HttpServerRequestSpan};
use crate::graphql::ObservedError;
use bytes::Bytes;
use http::header::{HOST, USER_AGENT};
use http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use http_body_util::Full;
use ntex::util::Bytes as NtexBytes;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::Span;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::Registry;

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

#[test]
fn test_http_server_request_span() {
    let layer = RecordingLayer::default();
    let subscriber = Registry::default().with(layer.clone());
    let response_body = "response body";

    with_default(subscriber, || {
        let req = ntex::web::test::TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(USER_AGENT, "test-agent")
            .to_http_request();
        let body = NtexBytes::from("test body");

        let span = HttpServerRequestSpan::from_request(&req, &body);
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

        let span = HttpServerRequestSpan::from_request(&req, &body);
        span.record_internal_server_error();

        layer.assert_recorded_value(&span, attributes::HTTP_RESPONSE_STATUS_CODE, "500");
        layer.assert_recorded_value(&span, attributes::OTEL_STATUS_CODE, "Error");
        layer.assert_recorded_value(&span, attributes::ERROR_TYPE, "500");
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
        let fingerprint: u64 = 12345;

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body, fingerprint);
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

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body, 1);
        span.record_as_leader();
        layer.assert_recorded_value(&span, attributes::HIVE_INFLIGHT_ROLE, "leader");

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body, 2);
        span.record_as_joiner();
        layer.assert_recorded_value(&span, attributes::HIVE_INFLIGHT_ROLE, "joiner");

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body, 3);
        span.record_response(&response_body, &StatusCode::OK);

        layer.assert_recorded_value(&span, attributes::HTTP_RESPONSE_STATUS_CODE, "200");
        layer.assert_recorded_value(
            &span,
            attributes::HTTP_RESPONSE_BODY_SIZE,
            &response_body.len().to_string(),
        );
        layer.assert_recorded_value(&span, attributes::OTEL_STATUS_CODE, "Ok");

        let span = HttpInflightRequestSpan::new(&method, &url, &headers, body, 4);
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
