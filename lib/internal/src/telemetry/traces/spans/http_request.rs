use bytes::Bytes;
use http::{
    header::{HOST, USER_AGENT},
    HeaderMap, Method, Response, StatusCode, Uri,
};
use http_body_util::Full;
use hyper::body::Body;
use ntex::http::body::MessageBody;
use std::borrow::{Borrow, Cow};
use tracing::{field::Empty, info_span, record_all, Level, Span};

use crate::telemetry::traces::{
    disabled_span, is_level_enabled,
    spans::{
        attributes::{self},
        kind::HiveSpanKind,
        TARGET_NAME,
    },
};

#[derive(Debug)]
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
    pub fn from_request(request: &ntex::web::HttpRequest) -> Self {
        if !is_level_enabled(Level::INFO) {
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

        let request_method = Cow::Borrowed(request.method());
        let header_user_agent = request.headers().get(USER_AGENT);
        let url = Cow::Borrowed(request.uri());
        let protocol_version = version_to_protocol_version_attr(request.version());

        // We follow the HTTP server span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-server
        let kind: &'static str = HiveSpanKind::HttpServerRequest.into();
        let span = info_span!(
            target: TARGET_NAME,
            "http.server",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Server",
            "error.type" = Empty,

            // Stable Attributes
            "server.address" = server_address,
            "server.port" = server_port,
            "url.full" = %url,
            "url.path" = url.path(),
            "url.scheme" = url.scheme_str(),
            "http.request.body.size" = Empty,
            "http.request.method" = request_method.as_str(),
            "network.protocol.version" = protocol_version,
            "user_agent.original" = header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
            "http.route" = url.path(),
        );

        Self { span }
    }

    pub fn record_body_size(&self, body_size: usize) {
        self.span
            .record(attributes::HTTP_REQUEST_BODY_SIZE, body_size);
    }

    pub fn record_response(&self, response: &ntex::web::HttpResponse) {
        if self.span.is_disabled() {
            return;
        }

        let body_size: Option<u64> = response.body().as_ref().and_then(|b| match b.size() {
            ntex::http::body::BodySize::Sized(size) => Some(size),
            _ => None,
        });

        record_all!(
            self.span,
            "http.response.status_code" = response.status().as_str(),
            "http.response.body.size" = body_size,
            "otel.status_code" = if response.status().is_server_error() {
                "Error"
            } else {
                "Ok"
            },
            "error.type" = if response.status().is_server_error() {
                Some(response.status().to_string())
            } else {
                None
            },
        );
    }

    pub fn record_internal_server_error(&self) {
        record_all!(
            self.span,
            "http.response.status_code" = 500,
            "otel.status_code" = "Error",
            "error.type" = 500,
        );
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
        if !is_level_enabled(Level::INFO) {
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

        // We follow the HTTP client span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-client
        let kind: &'static str = HiveSpanKind::HttpClientRequest.into();
        let span = info_span!(
            target: TARGET_NAME,
            "http.client",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Client",
            "error.type" = Empty,

            // Stable Attributes
            "server.address" = server_address,
            "server.port" = server_port,
            "url.full" = %url,
            "url.path" = url.path(),
            "url.scheme" = url.scheme_str(),
            "http.request.body.size" = request_body_size,
            "http.request.method" = request_method.as_str(),
            "network.protocol.version" = protocol_version,
            "user_agent.original" = header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
        );

        Self { span }
    }

    pub fn record_response<B>(&self, response: &Response<B>)
    where
        B: Body<Data = Bytes>,
    {
        if self.span.is_disabled() {
            return;
        }

        let body_size = response.body().size_hint().exact().map(|s| s as usize);

        record_all!(
            self.span,
            "http.response.status_code" = response.status().as_str(),
            "http.response.body.size" = body_size,
            "otel.status_code" = if response.status().is_server_error() {
                "Error"
            } else {
                "Ok"
            },
            "error.type" = if response.status().is_server_error() {
                Some(response.status().to_string())
            } else {
                None
            },
        );
    }

    pub fn record_internal_server_error(&self) {
        record_all!(
            self.span,
            "http.response.status_code" = 500,
            "otel.status_code" = "Error",
            "error.type" = 500,
        );
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
    pub fn new(method: &Method, url: &Uri, headers: &HeaderMap, body_bytes: &[u8]) -> Self {
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let server_address = url.host();
        let server_port = url.port_u16();

        let request_body_size = Some(body_bytes.len());
        let request_method = Cow::Borrowed(method);
        let header_user_agent = headers.get(USER_AGENT).map(Cow::Borrowed);
        let url = Cow::Borrowed(url);
        let protocol_version = version_to_protocol_version_attr(http::Version::HTTP_11);

        // We follow the HTTP client span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-client
        let kind: &'static str = HiveSpanKind::HttpInflightRequest.into();
        let span = info_span!(
            target: TARGET_NAME,
            "http.inflight",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,

            // Inflight Attributes
            "hive.inflight.role" = Empty,
            "hive.inflight.key" = Empty,

            // Stable Attributes
            "server.address" = server_address,
            "server.port" = server_port,
            "url.full" = %url,
            "url.path" = url.path(),
            "url.scheme" = url.scheme_str(),
            "http.request.body.size" = request_body_size,
            "http.request.method" = request_method.as_str(),
            "network.protocol.version" = protocol_version,
            "user_agent.original" = header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
        );

        Self { span }
    }

    pub fn record_as_leader(&self, leader_key: &u64) {
        record_all!(
            self.span,
            "hive.inflight.role" = "leader",
            "hive.inflight.key" = leader_key,
        );
    }

    pub fn record_as_joiner(&self, leader_key: &u64) {
        record_all!(
            self.span,
            "hive.inflight.role" = "joiner",
            "hive.inflight.key" = leader_key,
        );
    }

    pub fn record_response(&self, body: &Bytes, status: &StatusCode) {
        if self.span.is_disabled() {
            return;
        }

        let body_size = body.len();

        record_all!(
            self.span,
            "http.response.status_code" = status.as_str(),
            "http.response.body.size" = body_size as i64,
            "otel.status_code" = if status.is_server_error() {
                "Error"
            } else {
                "Ok"
            },
            "error.type" = if status.is_server_error() {
                Some(status.as_str())
            } else {
                None
            },
        );
    }

    pub fn record_internal_server_error(&self) {
        record_all!(
            self.span,
            "http.response.status_code" = 500,
            "otel.status_code" = "Error",
            "error.type" = 500,
        );
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
