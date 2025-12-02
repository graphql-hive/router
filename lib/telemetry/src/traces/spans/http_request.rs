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
            // We set both http.url (deprecated) and url.full (stable) for compatibility with different backends
            // OTel tools use those to display next to http client spans
            "http.url" = self.url.to_string(),
            "url.full" = self.url.to_string(),
            "url.path" = self.url.path(),
            "url.scheme" = self.url.scheme().map(|v| v.as_str()),
            "http.request.body.size" = self.request_body_size,
            "http.request.method" = self.request_method.as_str(),
            "network.protocol.version" = self.protocol_version,
            "user_agent.original" = self.header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
        );

        HttpServerRequestSpan { span }
    }
}

impl HttpServerRequestSpan {
    pub fn record_response(&self, response: &ntex::web::HttpResponse) {
        self.record("http.response.status_code", response.status().as_str());
        if let Some(body) = response.body().as_ref() {
            match body.size() {
                ntex::http::body::BodySize::None
                | ntex::http::body::BodySize::Empty
                | ntex::http::body::BodySize::Stream => {
                    self.record("http.response.body.size", 0);
                }
                ntex::http::body::BodySize::Sized(size) => {
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

        let span = info_span!(
            target: TARGET_NAME,
            "http.client",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Client",
            "error.type" = Empty,
            "server.address" = self.server_address,
            "server.port" = self.server_port,
            // We set both http.url (deprecated) and url.full (stable) for compatibility with different backends
            // OTel tools use those to display next to http client spans
            "http.url" = self.url.to_string(),
            "url.full" = self.url.to_string(),
            "url.path" = self.url.path(),
            "url.scheme" = self.url.scheme().map(|v| v.as_str()),
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
    pub fn record_response(&self, response: &Response<Incoming>) {
        self.record("http.response.status_code", response.status().as_str());
        if let Some(size) = response.body().size_hint().exact() {
            self.record("http.response.body.size", size);
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

fn version_to_protocol_version_attr(version: http::Version) -> Option<&'static str> {
    match version {
        http::Version::HTTP_10 => Some("1.0"),
        http::Version::HTTP_11 => Some("1.1"),
        http::Version::HTTP_2 => Some("2"),
        http::Version::HTTP_3 => Some("3"),
        _ => None,
    }
}

// Hive subgraph http
// SpanName: router.subgraph.request
// {
//   "busy_ns": "93875",
//   "code.file.path": "lib/telemetry/src/traces/spans/http_request.rs",
//   "code.line.number": "146",
//   "code.module.name": "hive_router_telemetry::traces::spans::http_request",
//   "hive.kind": "http.request",
//   "http.request.body.size": "404",
//   "http.request.method": "POST",
//   "idle_ns": "2458500",
//   "network.protocol.version": "1.1",
//   "target": "hive_router",
//   "thread.id": "8",
//   "thread.name": "worker:2",
//   "url.full": "http://0.0.0.0:4200/reviews",
//   "url.path": "/reviews",
//   "url.scheme": "http"
// }

// Cosmo
// {
//   "http.flavor": "1.1",
//   "http.method": "POST",
//   "http.request_content_length": "60",
//   "http.response_content_length": "400",
//   "http.status_code": "200",
//   "http.url": "http://localhost:4200/products",
//   "net.peer.name": "localhost",
//   "net.peer.port": "4200",
//   "wg.client.name": "unknown",
//   "wg.client.version": "missing",
//   "wg.component.name": "engine-transport",
//   "wg.operation.hash": "6710505681143383834",
//   "wg.operation.name": "TestQuery",
//   "wg.operation.protocol": "http",
//   "wg.operation.type": "query",
//   "wg.router.cluster.name": "",
//   "wg.router.config.version": "6e9dbed6-ec25-44f1-8693-4d44274fe9a6",
//   "wg.router.version": "0.247.0",
//   "wg.subgraph.id": "2",
//   "wg.subgraph.name": "products"
// }
