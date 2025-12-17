use bytes::Bytes;
use http::{
    header::{HOST, USER_AGENT},
    HeaderMap, Method, Response, StatusCode, Uri,
};
use http_body_util::Full;
use hyper::body::Body;
use ntex::http::body::MessageBody;
use std::borrow::{Borrow, Cow};
use tracing::{field::Empty, info_span, Span};

use crate::telemetry::traces::spans::{kind::HiveSpanKind, TARGET_NAME};

pub struct HttpServerRequestSpanBuilder<'a> {
    request_body_size: Option<usize>,
    request_method: Cow<'a, http::Method>,
    header_user_agent: Option<Cow<'a, http::HeaderValue>>,
    server_address: Option<&'a str>,
    server_port: Option<u16>,
    protocol_version: Option<&'a str>,
    url: Cow<'a, http::Uri>,
}

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

impl<'a> HttpServerRequestSpanBuilder<'a> {
    pub fn from_request(request: &'a ntex::web::HttpRequest, body: &ntex::util::Bytes) -> Self {
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
        HttpServerRequestSpanBuilder {
            request_body_size: Some(body.len()),
            request_method: Cow::Borrowed(request.method()),
            header_user_agent: request
                .headers()
                .get(USER_AGENT)
                .map(|h| Cow::Owned(h.into())),
            url: Cow::Borrowed(request.uri()),
            protocol_version: version_to_protocol_version_attr(request.version()),
            server_address,
            server_port,
        }
    }

    /// Consume self and turn into a [Span]
    pub fn build(self) -> HttpServerRequestSpan {
        // We follow the HTTP server span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-server
        let kind: &'static str = HiveSpanKind::HttpServerRequest.into();
        let url_full = self.url.to_string();

        let span = info_span!(
            target: TARGET_NAME,
            "http.server",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Server",
            "error.type" = Empty,

            // Stable Attributes
            "server.address" = self.server_address,
            "server.port" = self.server_port,
            "url.full" = url_full.as_str(),
            "url.path" = self.url.path(),
            "url.scheme" = self.url.scheme_str(),
            "http.request.body.size" = self.request_body_size,
            "http.request.method" = self.request_method.as_str(),
            "network.protocol.version" = self.protocol_version,
            "user_agent.original" = self.header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
            "http.route" = self.url.path(),
        );

        HttpServerRequestSpan { span }
    }
}

impl HttpServerRequestSpan {
    pub fn record_response(&self, response: &ntex::web::HttpResponse) {
        let body_size: Option<u64> = response.body().as_ref().and_then(|b| match b.size() {
            ntex::http::body::BodySize::Sized(size) => Some(size),
            _ => None,
        });

        // Record stable attributes
        self.record("http.response.status_code", response.status().as_str());
        if let Some(size) = body_size {
            self.record("http.response.body.size", size as i64);
        }

        if response.status().is_server_error() {
            self.record("otel.status_code", "Error");
            self.record("error.type", response.status().as_str());
        } else {
            self.record("otel.status_code", "Ok");
        }
    }

    pub fn record_internal_server_error(&self) {
        self.record("otel.status_code", "Error");
        self.record("error.type", "500");
        self.record("http.response.status_code", "500");
    }
}

pub struct HttpClientRequestSpanBuilder<'a> {
    request_body_size: Option<usize>,
    request_method: Cow<'a, http::Method>,
    header_user_agent: Option<Cow<'a, http::HeaderValue>>,
    server_address: Option<&'a str>,
    server_port: Option<u16>,
    protocol_version: Option<&'a str>,
    url: Cow<'a, http::Uri>,
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

impl<'a> HttpClientRequestSpanBuilder<'a> {
    pub fn from_request(request: &'a http::Request<Full<Bytes>>) -> Self {
        HttpClientRequestSpanBuilder {
            request_body_size: request.size_hint().upper().map(|v| v as usize),
            request_method: Cow::Borrowed(request.method()),
            header_user_agent: request.headers().get(USER_AGENT).map(Cow::Borrowed),
            url: Cow::Borrowed(request.uri()),
            protocol_version: version_to_protocol_version_attr(request.version()),
            server_address: request.uri().host(),
            server_port: request.uri().port_u16(),
        }
    }

    /// Consume self and turn into a [Span]
    pub fn build(self) -> HttpClientRequestSpan {
        // We follow the HTTP client span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-client
        let kind: &'static str = HiveSpanKind::HttpClientRequest.into();
        let url_full = self.url.to_string();

        let span = info_span!(
            target: TARGET_NAME,
            "http.client",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Client",
            "error.type" = Empty,

            // Stable Attributes
            "server.address" = self.server_address,
            "server.port" = self.server_port,
            "url.full" = url_full.as_str(),
            "url.path" = self.url.path(),
            "url.scheme" = self.url.scheme_str(),
            "http.request.body.size" = self.request_body_size,
            "http.request.method" = self.request_method.as_str(),
            "network.protocol.version" = self.protocol_version,
            "user_agent.original" = self.header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
        );

        HttpClientRequestSpan { span }
    }
}

impl HttpClientRequestSpan {
    pub fn record_response<B>(&self, response: &Response<B>)
    where
        B: Body<Data = Bytes>,
    {
        let body_size = response.body().size_hint().exact().map(|s| s as usize);

        // Record stable attributes
        self.record("http.response.status_code", response.status().as_str());
        if let Some(size) = body_size {
            self.record("http.response.body.size", size as i64);
        }

        if response.status().is_server_error() {
            self.record("otel.status_code", "Error");
            self.record("error.type", response.status().as_str());
        } else {
            self.record("otel.status_code", "Ok");
        }
    }

    pub fn record_internal_server_error(&self) {
        self.record("otel.status_code", "Error");
        self.record("error.type", "500");
        self.record("http.response.status_code", "500");
    }
}

pub struct HttpInflightRequestSpanBuilder<'a> {
    fingerprint: u64,
    request_body_size: Option<usize>,
    request_method: Cow<'a, http::Method>,
    header_user_agent: Option<Cow<'a, http::HeaderValue>>,
    server_address: Option<&'a str>,
    server_port: Option<u16>,
    protocol_version: Option<&'a str>,
    url: Cow<'a, http::Uri>,
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

impl<'a> HttpInflightRequestSpanBuilder<'a> {
    pub fn new(
        method: &'a Method,
        url: &'a Uri,
        headers: &'a HeaderMap,
        body_bytes: &[u8],
        fingerprint: u64,
    ) -> Self {
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

        HttpInflightRequestSpanBuilder {
            fingerprint,
            request_body_size: Some(body_bytes.len()),
            request_method: Cow::Borrowed(method),
            header_user_agent: headers.get(USER_AGENT).map(Cow::Borrowed),
            url: Cow::Borrowed(url),
            protocol_version: version_to_protocol_version_attr(http::Version::HTTP_11),
            server_address,
            server_port,
        }
    }

    /// Consume self and turn into a [Span]
    pub fn build(self) -> HttpInflightRequestSpan {
        // We follow the HTTP client span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-client
        let kind: &'static str = HiveSpanKind::HttpInflightRequest.into();
        let url_full = self.url.to_string();

        let span = info_span!(
            target: TARGET_NAME,
            "http.inflight",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,

            // Inflight Attributes
            "hive.inflight.role" = Empty,
            "hive.inflight.key" = self.fingerprint,

            // Stable Attributes
            "server.address" = self.server_address,
            "server.port" = self.server_port,
            "url.full" = url_full.as_str(),
            "url.path" = self.url.path(),
            "url.scheme" = self.url.scheme_str(),
            "http.request.body.size" = self.request_body_size,
            "http.request.method" = self.request_method.as_str(),
            "network.protocol.version" = self.protocol_version,
            "user_agent.original" = self.header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
        );

        HttpInflightRequestSpan { span }
    }
}

impl HttpInflightRequestSpan {
    pub fn record_as_leader(&self) {
        self.record("hive.inflight.role", "leader");
    }

    pub fn record_as_joiner(&self) {
        self.record("hive.inflight.role", "joiner");
    }

    pub fn record_response(&self, body: &Bytes, status: &StatusCode) {
        let body_size = body.len();

        // Record stable attributes
        self.record("http.response.status_code", status.as_str());
        self.record("http.response.body.size", body_size as i64);

        if status.is_server_error() {
            self.record("otel.status_code", "Error");
            self.record("error.type", status.as_str());
        } else {
            self.record("otel.status_code", "Ok");
        }
    }

    pub fn record_internal_server_error(&self) {
        self.record("otel.status_code", "Error");
        self.record("error.type", "500");
        self.record("http.response.status_code", "500");
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
