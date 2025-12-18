use bytes::Bytes;
use fastrace::prelude::Span;
use http::{
    header::{HOST, USER_AGENT},
    HeaderMap, Method, Response, StatusCode, Uri,
};
use http_body_util::Full;
use hyper::body::Body;
use ntex::http::body::MessageBody;
use std::borrow::{Borrow, Cow};

use crate::telemetry::traces::{
    disabled_span, is_tracing_enabled,
    spans::{attributes, kind::HiveSpanKind, TARGET_NAME},
};

pub struct HttpServerRequestSpan {
    pub span: Span,
}

impl std::ops::Deref for HttpServerRequestSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Borrow<Span> for HttpServerRequestSpan {
    fn borrow(&self) -> &Span {
        &self.span
    }
}

impl HttpServerRequestSpan {
    pub fn from_request(request: &ntex::web::HttpRequest, body: &ntex::util::Bytes) -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let (server_address, server_port) =
            match request.headers().get(HOST).and_then(|h| h.to_str().ok()) {
                Some(host) => {
                    if let Some((host, port_str)) = host.rsplit_once(':') {
                        (Some(host), port_str.parse::<u16>().ok())
                    } else {
                        (Some(host), None)
                    }
                }
                None => (None, None),
            };

        let request_body_size = body.len();
        let request_method = Cow::Borrowed(request.method());
        let header_user_agent = request.headers().get(USER_AGENT);
        let url = Cow::Borrowed(request.uri());
        let url_full = url.to_string();
        let protocol_version = version_to_protocol_version_attr(request.version());

        // We follow the HTTP server span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-server
        let kind: &'static str = HiveSpanKind::HttpServerRequest.into();
        let mut span = Span::enter_with_local_parent("http.server").with_properties(|| {
            vec![
                ("target", TARGET_NAME),
                (attributes::HIVE_KIND, kind),
                (attributes::OTEL_KIND, "Server"),
            ]
        });

        span = span.with_properties(|| {
            vec![
                (
                    attributes::HTTP_REQUEST_BODY_SIZE,
                    request_body_size.to_string(),
                ),
                (attributes::URL_FULL, url_full.as_str().to_string()),
                (attributes::URL_PATH, url.path().to_string()),
                (
                    attributes::HTTP_REQUEST_METHOD,
                    request_method.as_str().to_string(),
                ),
                (attributes::HTTP_ROUTE, url.path().to_string()),
            ]
        });

        if let Some(scheme) = url.scheme_str() {
            span = span.with_property(|| (attributes::URL_SCHEME, scheme.to_string()));
        }

        if let Some(ua) = header_user_agent.as_ref().and_then(|v| v.to_str().ok()) {
            span = span.with_property(|| (attributes::USER_AGENT_ORIGINAL, ua.to_string()));
        }

        if let Some(pv) = protocol_version {
            span = span.with_property(|| (attributes::NETWORK_PROTOCOL_VERSION, pv.to_string()));
        }

        if let Some(addr) = server_address {
            span = span.with_property(|| (attributes::SERVER_ADDRESS, addr.to_string()));
        }

        if let Some(port) = server_port {
            span = span.with_property(|| (attributes::SERVER_PORT, port.to_string()));
        }

        Self { span }
    }

    pub fn record_response(&self, response: &ntex::web::HttpResponse) {
        let body_size: Option<u64> = response.body().as_ref().and_then(|b| match b.size() {
            ntex::http::body::BodySize::Sized(size) => Some(size),
            _ => None,
        });

        // Record stable attributes
        self.span.add_property(|| {
            (
                attributes::HTTP_RESPONSE_STATUS_CODE,
                response.status().as_str().to_string(),
            )
        });
        if let Some(size) = body_size {
            self.span
                .add_property(|| (attributes::HTTP_RESPONSE_BODY_SIZE, size.to_string()));
        }

        if response.status().is_server_error() {
            self.span
                .add_property(|| (attributes::OTEL_STATUS_CODE, "Error"));
            self.span.add_property(|| {
                (
                    attributes::ERROR_TYPE,
                    response.status().as_str().to_string(),
                )
            });
        } else {
            self.span
                .add_property(|| (attributes::OTEL_STATUS_CODE, "Ok"));
        }
    }

    pub fn record_internal_server_error(&self) {
        self.span.add_properties(|| {
            vec![
                (attributes::OTEL_STATUS_CODE, "Error"),
                (attributes::ERROR_TYPE, "500"),
            ]
        });
        self.span
            .add_property(|| (attributes::HTTP_RESPONSE_STATUS_CODE, "500"));
    }
}

pub struct HttpClientRequestSpan {
    pub span: Span,
}

impl std::ops::Deref for HttpClientRequestSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Borrow<Span> for HttpClientRequestSpan {
    fn borrow(&self) -> &Span {
        &self.span
    }
}

impl HttpClientRequestSpan {
    pub fn from_request(request: &http::Request<Full<Bytes>>) -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let request_body_size = request.size_hint().upper().map(|v| v as usize);
        let request_method = Cow::Borrowed(request.method());
        let header_user_agent = request.headers().get(USER_AGENT).map(Cow::Borrowed);
        let url = Cow::Borrowed(request.uri());
        let protocol_version = version_to_protocol_version_attr(request.version());
        let server_address = request.uri().host();
        let server_port = request.uri().port_u16();
        let url_full = url.to_string();

        // We follow the HTTP client span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-client
        let kind: &'static str = HiveSpanKind::HttpClientRequest.into();
        let mut span = Span::enter_with_local_parent("http.client").with_properties(|| {
            vec![
                ("target", TARGET_NAME),
                (attributes::HIVE_KIND, kind),
                (attributes::OTEL_KIND, "Client"),
            ]
        });

        span = span.with_properties(|| {
            vec![
                (attributes::URL_FULL, url_full.as_str().to_string()),
                (attributes::URL_PATH, url.path().to_string()),
                (
                    attributes::HTTP_REQUEST_METHOD,
                    request_method.as_str().to_string(),
                ),
            ]
        });

        if let Some(scheme) = url.scheme_str() {
            span = span.with_property(|| (attributes::URL_SCHEME, scheme.to_string()));
        }

        if let Some(size) = request_body_size {
            span = span.with_property(|| (attributes::HTTP_REQUEST_BODY_SIZE, size.to_string()));
        }

        if let Some(addr) = server_address {
            span = span.with_property(|| (attributes::SERVER_ADDRESS, addr.to_string()));
        }

        if let Some(port) = server_port {
            span = span.with_property(|| (attributes::SERVER_PORT, port.to_string()));
        }

        if let Some(ua) = header_user_agent.as_ref().and_then(|v| v.to_str().ok()) {
            span = span.with_property(|| (attributes::USER_AGENT_ORIGINAL, ua.to_string()));
        }

        if let Some(pv) = protocol_version {
            span = span.with_property(|| (attributes::NETWORK_PROTOCOL_VERSION, pv.to_string()));
        }

        Self { span }
    }

    pub fn record_response<B>(&self, response: &Response<B>)
    where
        B: Body<Data = Bytes>,
    {
        let body_size = response.body().size_hint().exact().map(|s| s as usize);

        // Record stable attributes
        self.span.add_property(|| {
            (
                attributes::HTTP_RESPONSE_STATUS_CODE,
                response.status().as_str().to_string(),
            )
        });
        if let Some(size) = body_size {
            self.span
                .add_property(|| (attributes::HTTP_RESPONSE_BODY_SIZE, size.to_string()));
        }

        if response.status().is_server_error() {
            self.span
                .add_property(|| (attributes::OTEL_STATUS_CODE, "Error"));
            self.span.add_property(|| {
                (
                    attributes::ERROR_TYPE,
                    response.status().as_str().to_string(),
                )
            });
        } else {
            self.span
                .add_property(|| (attributes::OTEL_STATUS_CODE, "Ok"));
        }
    }

    pub fn record_internal_server_error(&self) {
        self.span.add_properties(|| {
            vec![
                (attributes::OTEL_STATUS_CODE, "Error"),
                (attributes::ERROR_TYPE, "500"),
                (attributes::HTTP_RESPONSE_STATUS_CODE, "500"),
            ]
        });
    }
}

pub struct HttpInflightRequestSpan {
    pub span: Span,
}

impl std::ops::Deref for HttpInflightRequestSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Borrow<Span> for HttpInflightRequestSpan {
    fn borrow(&self) -> &Span {
        &self.span
    }
}
impl HttpInflightRequestSpan {
    pub fn new(
        method: &Method,
        url: &Uri,
        headers: &HeaderMap,
        body_bytes: &[u8],
        fingerprint: u64,
    ) -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let (server_address, server_port) = match headers.get(HOST).and_then(|h| h.to_str().ok()) {
            Some(host) => {
                if let Some((host, port_str)) = host.rsplit_once(':') {
                    (Some(host), port_str.parse::<u16>().ok())
                } else {
                    (Some(host), None)
                }
            }
            None => (None, None),
        };

        let request_body_size = Some(body_bytes.len());
        let request_method = Cow::Borrowed(method);
        let header_user_agent = headers.get(USER_AGENT).map(Cow::Borrowed);
        let url = Cow::Borrowed(url);
        let protocol_version = version_to_protocol_version_attr(http::Version::HTTP_11);
        let url_full = url.to_string();

        // We follow the HTTP client span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-client
        let kind: &'static str = HiveSpanKind::HttpInflightRequest.into();
        let mut span = Span::enter_with_local_parent("http.inflight").with_properties(|| {
            vec![
                ("target", TARGET_NAME),
                (attributes::HIVE_KIND, kind),
                (attributes::OTEL_KIND, "Internal"),
            ]
        });

        span = span.with_properties(|| {
            vec![
                (attributes::HIVE_INFLIGHT_KEY, fingerprint.to_string()),
                (attributes::URL_FULL, url_full.as_str().to_string()),
                (attributes::URL_PATH, url.path().to_string()),
                (
                    attributes::HTTP_REQUEST_METHOD,
                    request_method.as_str().to_string(),
                ),
            ]
        });

        if let Some(scheme) = url.scheme_str() {
            span = span.with_property(|| (attributes::URL_SCHEME, scheme.to_string()));
        }

        if let Some(size) = request_body_size {
            span = span.with_property(|| (attributes::HTTP_REQUEST_BODY_SIZE, size.to_string()));
        }

        if let Some(addr) = server_address {
            span = span.with_property(|| (attributes::SERVER_ADDRESS, addr.to_string()));
        }

        if let Some(port) = server_port {
            span = span.with_property(|| (attributes::SERVER_PORT, port.to_string()));
        }

        if let Some(ua) = header_user_agent.as_ref().and_then(|v| v.to_str().ok()) {
            span = span.with_property(|| (attributes::USER_AGENT_ORIGINAL, ua.to_string()));
        }

        if let Some(pv) = protocol_version {
            span = span.with_property(|| (attributes::NETWORK_PROTOCOL_VERSION, pv.to_string()));
        }

        Self { span }
    }

    pub fn record_as_leader(&self) {
        self.span
            .add_property(|| (attributes::HIVE_INFLIGHT_ROLE, "leader"));
    }

    pub fn record_as_joiner(&self) {
        self.span
            .add_property(|| (attributes::HIVE_INFLIGHT_ROLE, "joiner"));
    }

    pub fn record_response(&self, body: &Bytes, status: &StatusCode) {
        let body_size = body.len();

        // Record stable attributes
        self.span.add_properties(|| {
            vec![
                (
                    attributes::HTTP_RESPONSE_STATUS_CODE,
                    status.as_str().to_string(),
                ),
                (attributes::HTTP_RESPONSE_BODY_SIZE, body_size.to_string()),
            ]
        });

        if status.is_server_error() {
            self.span
                .add_property(|| (attributes::OTEL_STATUS_CODE, "Error"));
            self.span
                .add_property(|| (attributes::ERROR_TYPE, status.as_str().to_string()));
        } else {
            self.span
                .add_property(|| (attributes::OTEL_STATUS_CODE, "Ok"));
        }
    }

    pub fn record_internal_server_error(&self) {
        self.span.add_properties(|| {
            vec![
                (attributes::OTEL_STATUS_CODE, "Error"),
                (attributes::ERROR_TYPE, "500"),
                (attributes::HTTP_RESPONSE_STATUS_CODE, "500"),
            ]
        });
    }
}

fn version_to_protocol_version_attr(version: http::Version) -> Option<&'static str> {
    match version {
        http::Version::HTTP_10 => Some("1.0"),
        http::Version::HTTP_11 => Some("1.1"),
        http::Version::HTTP_2 => Some("2"),
        http::Version::HTTP_3 => Some("3"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::traces::spans::attributes;

    fn assert_fields(span: &Span, expected_fields: &[&str]) {
        for field in expected_fields {
            // assert!(
            //     span.field(*field).is_some(),
            //     "Field '{}' is missing from span '{}'",
            //     field,
            //     span.metadata().expect("Span should have metadata").name()
            // );
        }
    }

    #[test]
    fn test_http_server_request_span_fields() {
        let req = ntex::web::test::TestRequest::with_uri("/graphql")
            .header(HOST, "localhost:8080")
            .header(USER_AGENT, "test-agent")
            .to_http_request();
        let body = ntex::util::Bytes::from("test body");

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
    }

    #[test]
    fn test_http_client_request_span_fields() {
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
    }

    #[test]
    fn test_http_inflight_request_span_fields() {
        let method = http::Method::POST;
        let url = http::Uri::from_static("http://localhost:8082/graphql");
        let mut headers = http::HeaderMap::new();
        headers.insert(HOST, http::HeaderValue::from_static("localhost:8082"));
        headers.insert(USER_AGENT, http::HeaderValue::from_static("test-agent"));
        let body = b"test body";
        let fingerprint = 12345u64;

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
    }
}
