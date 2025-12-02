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

pub const ROUTER_HTTP_REQUEST_SPAN_NAME: &str = "router.request";
pub const SUBGRAPH_HTTP_REQUEST_SPAN_NAME: &str = "router.subgraph.request";

enum HttpRequestSpanType {
    Router,
    Subgraph,
}

pub struct HttpRequestSpanBuilder<'a> {
    kind: HttpRequestSpanType,
    request_body_size: Option<usize>,
    request_method: Cow<'a, http::Method>,
    header_user_agent: Option<Cow<'a, http::HeaderValue>>,
    server_address: Option<&'a str>,
    server_port: Option<u16>,
    protocol_version: Option<&'a str>,
    url: Cow<'a, http::Uri>,
}

#[derive(Clone)]
pub struct HttpRequestSpan {
    pub span: Span,
}

impl std::ops::Deref for HttpRequestSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Borrow<Span> for HttpRequestSpan {
    fn borrow(&self) -> &Span {
        &self.span
    }
}

impl<'a> HttpRequestSpanBuilder<'a> {
    pub fn from_subgraph_request(request: &'a http::Request<Full<Bytes>>) -> Self {
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

        HttpRequestSpanBuilder {
            kind: HttpRequestSpanType::Subgraph,
            request_body_size: request.size_hint().upper().map(|v| v as usize),
            request_method: Cow::Borrowed(request.method()),
            header_user_agent: request.headers().get(USER_AGENT).map(Cow::Borrowed),
            url: Cow::Borrowed(request.uri()),
            protocol_version: version_to_protocol_version_attr(request.version()),
            server_address,
            server_port,
        }
    }

    pub fn from_router_request(
        request: &'a ntex::web::HttpRequest,
        body: &ntex::util::Bytes,
    ) -> Self {
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
        HttpRequestSpanBuilder {
            kind: HttpRequestSpanType::Router,
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
    pub fn build(self) -> HttpRequestSpan {
        // We follow the HTTP server span conventions:
        // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-server
        let kind: &'static str = HiveSpanKind::HttpRequest.into();

        // Macro to reduce code duplication.
        // Rust complains about span's name not being constant,
        // so assinging span's name dynamically is not possible without macro.
        macro_rules! build_http_request_span {
          ($span_name:expr, $otel_kind:expr) => {
            info_span!(
                target: TARGET_NAME,
                $span_name,
                "hive.kind" = kind,
                "otel.status_code" = Empty,
                "otel.kind" = $otel_kind,
                "error.type" = Empty,
                "server.address" = self.server_address,
                "server.port" = self.server_port,
                "url.full" = self.url.to_string(),
                "url.path" = self.url.path(),
                "url.scheme" = self.url.scheme().map(|v| v.as_str()),
                "http.request.body.size" = self.request_body_size,
                "http.request.method" = self.request_method.as_str(),
                "network.protocol.version" = self.protocol_version,
                "user_agent.original" = self.header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
                "http.response.status_code" = Empty,
                "http.response.body.size" = Empty,
            )
          };
        }

        let span = match self.kind {
            HttpRequestSpanType::Router => {
                build_http_request_span!(ROUTER_HTTP_REQUEST_SPAN_NAME, "Server")
            }
            HttpRequestSpanType::Subgraph => {
                build_http_request_span!(SUBGRAPH_HTTP_REQUEST_SPAN_NAME, "Client")
            }
        };

        HttpRequestSpan { span }
    }
}

impl HttpRequestSpan {
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

    pub fn record_ntex_response(&self, response: &ntex::web::HttpResponse) {
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

fn version_to_protocol_version_attr(version: http::Version) -> Option<&'static str> {
    match version {
        http::Version::HTTP_10 => Some("1.0"),
        http::Version::HTTP_11 => Some("1.1"),
        http::Version::HTTP_2 => Some("2"),
        http::Version::HTTP_3 => Some("3"),
        _ => None,
    }
}
