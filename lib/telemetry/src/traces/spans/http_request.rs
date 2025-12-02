use bytes::Bytes;
use http::{
    header::{HOST, USER_AGENT},
    Response,
};
use http_body_util::Full;
use hyper::body::{Body, Incoming};
use ntex::http::body::MessageBody;
use std::borrow::{Borrow, Cow};

use tracing::{field::Empty, info_span, Span};

use crate::traces::spans::{kind::HiveSpanKind, TARGET_NAME};

pub struct HttpServerRequestSpanBuilder<'a> {
    request_body_size: Option<usize>,
    request_method: Cow<'a, http::Method>,
    header_user_agent: Option<Cow<'a, http::HeaderValue>>,
    server_address: Option<&'a str>,
    server_port: Option<u16>,
    protocol_version: Option<&'a str>,
    url: Cow<'a, http::Uri>,
}

#[derive(Clone)]
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
            "server.address" = self.server_address,
            "server.port" = self.server_port,
            "http.route" = "/graphql",
            "url.full" = url_full,
            "url.path" = self.url.path(),
            "url.scheme" = self.url.scheme().map(|v| v.as_str()),
            "http.request.body.size" = self.request_body_size,
            "http.request.method" = self.request_method.as_str(),
            "network.protocol.version" = self.protocol_version,
            "user_agent.original" = self.header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
        );

        record_deprecated_http_server_request_attributes(&span, &self, &url_full);

        HttpServerRequestSpan { span }
    }
}

impl HttpServerRequestSpan {
    pub fn record_response(&self, response: &ntex::web::HttpResponse) {
        let mut body_size: Option<u64> = None;

        self.record("http.response.status_code", response.status().as_str());
        if let Some(body) = response.body().as_ref() {
            match body.size() {
                ntex::http::body::BodySize::None
                | ntex::http::body::BodySize::Empty
                | ntex::http::body::BodySize::Stream => {
                    body_size = Some(0);
                    self.record("http.response.body.size", 0);
                }
                ntex::http::body::BodySize::Sized(size) => {
                    body_size = Some(size);
                    self.record("http.response.body.size", size);
                }
            }
        }
        if response.status().is_server_error() {
            self.record("otel.status_code", "Error");
            self.record("error.type", response.status().as_str());
        } else {
            self.record("otel.status_code", "Ok");
        }

        record_deprecated_http_response_attributes(
            &self.span,
            response.status().as_str(),
            body_size,
        );
    }

    pub fn record_internal_server_error(&self) {
        self.record("otel.status_code", "Error");
        self.record("error.type", "500");
        self.record("http.response.status_code", "500");
        record_deprecated_http_response_attributes(&self.span, "500", None);
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

#[derive(Clone)]
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

        HttpClientRequestSpanBuilder {
            request_body_size: request.size_hint().upper().map(|v| v as usize),
            request_method: Cow::Borrowed(request.method()),
            header_user_agent: request.headers().get(USER_AGENT).map(Cow::Borrowed),
            url: Cow::Borrowed(request.uri()),
            protocol_version: version_to_protocol_version_attr(request.version()),
            server_address,
            server_port,
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
            "server.address" = self.server_address,
            "server.port" = self.server_port,
            "url.full" = url_full,
            "url.path" = self.url.path(),
            "url.scheme" = self.url.scheme().map(|v| v.as_str()),
            "http.request.body.size" = self.request_body_size,
            "http.request.method" = self.request_method.as_str(),
            "network.protocol.version" = self.protocol_version,
            "user_agent.original" = self.header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
        );

        record_deprecated_http_client_request_attributes(&span, &self, &url_full);

        HttpClientRequestSpan { span }
    }
}

impl HttpClientRequestSpan {
    pub fn record_response(&self, response: &Response<Incoming>) {
        self.record("http.response.status_code", response.status().as_str());

        let mut body_size: Option<u64> = None;

        if let Some(size) = response.body().size_hint().exact() {
            body_size = Some(size);
            self.record("http.response.body.size", size);
        }
        if response.status().is_server_error() {
            self.record("otel.status_code", "Error");
            self.record("error.type", response.status().as_str());
        } else {
            self.record("otel.status_code", "Ok");
        }

        record_deprecated_http_response_attributes(
            &self.span,
            response.status().as_str(),
            body_size,
        );
    }

    pub fn record_internal_server_error(&self) {
        self.record("otel.status_code", "Error");
        self.record("error.type", "500");
        self.record("http.response.status_code", "500");

        record_deprecated_http_response_attributes(&self.span, "500", None);
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

/// Records deprecated HTTP server attributes onto a span for backwards compatibility.
/// TODO: make it opt-in or opt-out
fn record_deprecated_http_server_request_attributes(
    span: &Span,
    builder: &HttpServerRequestSpanBuilder<'_>,
    full_url: &str,
) {
    span.record("http.method", builder.request_method.as_str());
    span.record("http.url", full_url);
    span.record("http.host", builder.server_address);
    span.record("http.scheme", builder.url.scheme_str());
    span.record("http.flavor", builder.protocol_version);
    if let Some(size) = builder.request_body_size {
        span.record("http.request_content_length", size as i64);
    }
    if let Some(ua) = builder
        .header_user_agent
        .as_ref()
        .and_then(|v| v.to_str().ok())
    {
        span.record("http.user_agent", ua);
    }
    if let Some(path_and_query) = builder.url.path_and_query() {
        span.record("http.target", path_and_query.as_str());
    }
}

/// Records deprecated HTTP client attributes onto a span for backwards compatibility.
/// TODO: make it opt-in or opt-out
fn record_deprecated_http_client_request_attributes(
    span: &Span,
    builder: &HttpClientRequestSpanBuilder<'_>,
    full_url: &str,
) {
    span.record("http.method", builder.request_method.as_str());
    span.record("http.url", full_url);
    span.record("net.peer.name", builder.server_address);
    span.record("net.peer.port", builder.server_port);
    span.record("http.flavor", builder.protocol_version);
    if let Some(size) = builder.request_body_size {
        span.record("http.request_content_length", size as i64);
    }
}

/// Records deprecated HTTP response attributes onto a span for backwards compatibility.
/// TODO: make it opt-in or opt-out
fn record_deprecated_http_response_attributes(
    span: &Span,
    status_code: &str,
    body_size: Option<u64>,
) {
    span.record("http.status_code", status_code);
    if let Some(size) = body_size {
        span.record("http.response_content_length", size as i64);
    }
}
