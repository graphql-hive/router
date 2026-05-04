use bytes::Bytes;
use hive_router_config::{primitives::ip_network::IpNetwork, telemetry::ClientIpHeaderConfig};
use http::{
    header::{FORWARDED, HOST, USER_AGENT},
    HeaderMap, HeaderName, Method, Response, StatusCode, Uri, Version,
};
use http_body_util::Full;
use hyper::body::Body;
use ntex::http::{body::MessageBody, HeaderMap as NtexHeaderMap};
use std::borrow::{Borrow, Cow};
use std::net::{IpAddr, SocketAddr};
use tracing::{field::Empty, info_span, record_all, Level, Span};

/// Minimal request interface required to build an HTTP server span.
///
/// This keeps `HttpServerRequestSpan::from_request` decoupled from
/// `ntex::web::HttpRequest` and allows tests to provide a mock request carrying
/// a peer address.
pub trait HttpServerSpanRequest {
    fn headers(&self) -> &NtexHeaderMap;
    fn method(&self) -> &Method;
    fn uri(&self) -> &Uri;
    fn version(&self) -> Version;
    fn peer_addr(&self) -> Option<SocketAddr>;
}

impl HttpServerSpanRequest for ntex::web::HttpRequest {
    fn headers(&self) -> &NtexHeaderMap {
        self.headers()
    }

    fn method(&self) -> &Method {
        self.method()
    }

    fn uri(&self) -> &Uri {
        self.uri()
    }

    fn version(&self) -> Version {
        self.version()
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr()
    }
}

use crate::http::{HttpMethodAsStr, HttpUriAsStr, HttpVersionAsStr};
use crate::telemetry::traces::{
    disabled_span, is_level_enabled,
    spans::{
        attributes::{self},
        kind::HiveSpanKind,
        TARGET_NAME,
    },
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
    pub fn from_request<Req: HttpServerSpanRequest>(
        request: &Req,
        client_ip_header_config: &Option<ClientIpHeaderConfig>,
    ) -> Self {
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

        let request_method = request.method().as_static_str();
        let header_user_agent = request.headers().get(USER_AGENT);
        let url = Cow::Borrowed(request.uri());
        let protocol_version = request.version().as_static_str();
        let url_scheme = url.scheme_static_str();
        let peer = request.peer_addr();
        let peer_address = peer.map(|p| p.ip().to_string());
        let peer_port = peer.map(|p| p.port());
        let (client_address, client_port) =
            match resolve_client_address(request, client_ip_header_config) {
                Some(client) => (Some(client.ip_raw.to_string()), client.port),
                None => (peer_address.clone(), peer_port),
            };

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
            "url.scheme" = url_scheme,
            "http.request.body.size" = Empty,
            "http.request.method" = request_method,
            "network.protocol.version" = protocol_version,
            "user_agent.original" = header_user_agent.as_ref().and_then(|v| v.to_str().ok()),
            "http.response.status_code" = Empty,
            "http.response.body.size" = Empty,
            "http.route" = url.path(),
            // Client
            "client.address" = client_address,
            "client.port" = client_port,
            "network.peer.address" = peer_address,
            "network.peer.port" = peer_port,
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
        let request_method = request.method().as_static_str();
        let header_user_agent = request.headers().get(USER_AGENT).map(Cow::Borrowed);
        let url = Cow::Borrowed(request.uri());
        let protocol_version = request.version().as_static_str();
        let server_address = request.uri().host();
        let server_port = request.uri().port_u16();
        let url_scheme = request.uri().scheme_static_str();

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
            "url.scheme" = url_scheme,
            "http.request.body.size" = request_body_size,
            "http.request.method" = request_method,
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
        let request_method = method.as_static_str();
        let header_user_agent = headers.get(USER_AGENT).map(Cow::Borrowed);
        let url = Cow::Borrowed(url);
        let protocol_version = http::Version::HTTP_11.as_static_str();
        let url_scheme = url.scheme_static_str();

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
            "url.scheme" = url_scheme,
            "http.request.body.size" = request_body_size,
            "http.request.method" = request_method,
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

fn resolve_client_address<'a, Req: HttpServerSpanRequest>(
    request: &'a Req,
    config: &Option<ClientIpHeaderConfig>,
) -> Option<ParsedAddr<'a>> {
    let config = config.as_ref()?;

    match config {
        ClientIpHeaderConfig::HeaderName(name) => {
            let header = request
                .headers()
                .get(name.get_header_ref())?
                .to_str()
                .ok()?;

            // Finds the left-most valid address
            ParsedAddr::from(name.get_header_ref(), header).next()
        }

        ClientIpHeaderConfig::TrustedProxies(cfg) => {
            let peer_ip = request.peer_addr()?.ip();

            // If the peer IP is not trusted, then we can't determine the client address
            if !ParsedAddr::is_trusted(peer_ip, &cfg.trusted_proxies) {
                return None;
            }

            let header = request
                .headers()
                .get(cfg.name.get_header_ref())?
                .to_str()
                .ok()?;

            let mut addrs = ParsedAddr::from(cfg.name.get_header_ref(), header);
            let first = addrs.next()?;

            addrs
                // We go from right to left
                .rev()
                // to find the first non-trusted address
                .find(|addr| !ParsedAddr::is_trusted(addr.parsed_ip, &cfg.trusted_proxies))
                // and all are trusted, we treat the first as the client address
                .unwrap_or(first)
                .into()
        }
    }
}

#[derive(Clone, Copy)]
struct ParsedAddr<'a> {
    // We keep the str version of IP to avoid the cost of `Display::fmt` of IpAddr
    ip_raw: &'a str,
    parsed_ip: IpAddr,
    port: Option<u16>,
}

impl<'a> ParsedAddr<'a> {
    #[inline]
    fn parse_address(raw: &str) -> Option<ParsedAddr<'_>> {
        let raw = raw.trim();

        if raw.is_empty() {
            return None;
        }

        // Normal IP - `192.168.1.1` or `::1`
        // We aim to find it first, as it's the most common case.
        if let Ok(ip) = raw.parse::<IpAddr>() {
            return Some(ParsedAddr {
                ip_raw: raw,
                parsed_ip: ip,
                port: None,
            });
        }

        // IPv6 with brackets - `[::1]` or `[::1]:8080`
        if let Some(rest) = raw.strip_prefix('[') {
            let (addr, rest) = rest.split_once(']')?;

            let port = rest
                .strip_prefix(':')
                .map(str::parse::<u16>)
                .transpose()
                .ok()?;

            return Some(ParsedAddr {
                ip_raw: addr,
                parsed_ip: addr.parse().ok()?,
                port,
            });
        }

        // IPv4 with port
        let (addr, port) = raw.rsplit_once(':')?;

        // If the address contains `:`, then it's not a valid IPv4 or IPv6 address.
        if addr.contains(':') {
            return None;
        }

        Some(ParsedAddr {
            ip_raw: addr,
            parsed_ip: addr.parse().ok()?,
            port: Some(port.parse().ok()?),
        })
    }

    #[inline]
    fn from(
        name: &HeaderName,
        value: &'a str,
    ) -> impl DoubleEndedIterator<Item = ParsedAddr<'a>> + 'a {
        let is_forwarded = name == FORWARDED;

        value.split(',').filter_map(move |part| {
            let value = if is_forwarded {
                // Forwarded header values are of the form:
                // `for=client-ip;proto=http;by=proxy-ip`
                part.split(';').find_map(|kv| {
                    let (key, value) = kv.trim().split_once('=')?;

                    key.eq_ignore_ascii_case("for")
                        .then_some(value.trim().trim_matches('"'))
                })?
            } else {
                part
            };

            Self::parse_address(value)
        })
    }

    #[inline]
    fn is_trusted(ip: IpAddr, proxies: &[IpNetwork]) -> bool {
        proxies.iter().any(|network| network.contains(&ip))
    }
}
