//! https://opentelemetry.io/docs/specs/semconv/http/http-metrics
use std::time::Instant;

use ntex::http::body::{BodySize, MessageBody};
use ntex::web::{HttpRequest, HttpResponse};
use opentelemetry::{metrics::Histogram, metrics::Meter, metrics::UpDownCounter, KeyValue};

use crate::http::{HttpMethodAsStr, HttpUriAsStr, HttpVersionAsStr};
use crate::telemetry::metrics::capture::Capture;
#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names, values};

struct HttpServerInstruments {
    request_duration: Option<Histogram<f64>>,
    active_requests: Option<UpDownCounter<i64>>,
    request_body_size: Option<Histogram<u64>>,
    response_body_size: Option<Histogram<u64>>,
}

impl HttpServerInstruments {
    fn is_enabled(&self) -> bool {
        self.request_duration.is_some()
            || self.active_requests.is_some()
            || self.request_body_size.is_some()
            || self.response_body_size.is_some()
    }
}

pub struct HttpServerRequestState<'a> {
    instruments: &'a HttpServerInstruments,
    _active_request_guard: HttpServerActiveRequestGuard<'a>,
    method: &'static str,
    scheme: &'static str,
    route: String,
    protocol_version: &'static str,
    started_at: Instant,
}

pub struct HttpServerActiveRequestGuard<'a> {
    instruments: &'a HttpServerInstruments,
    method: &'static str,
    scheme: &'static str,
}

pub struct HttpServerMetrics {
    instruments: HttpServerInstruments,
}

impl HttpServerMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let request_duration = meter.map(|meter| {
            meter
                .f64_histogram(names::HTTP_SERVER_REQUEST_DURATION)
                .with_unit("s")
                .with_description("Duration of HTTP server requests")
                .build()
        });

        let request_body_size = meter.map(|meter| {
            meter
                .u64_histogram(names::HTTP_SERVER_REQUEST_BODY_SIZE)
                .with_unit("By")
                .with_description("Size of HTTP server request bodies")
                .build()
        });

        let response_body_size = meter.map(|meter| {
            meter
                .u64_histogram(names::HTTP_SERVER_RESPONSE_BODY_SIZE)
                .with_unit("By")
                .with_description("Size of HTTP server response bodies")
                .build()
        });

        let active_requests = meter.map(|meter| {
            meter
                .i64_up_down_counter(names::HTTP_SERVER_ACTIVE_REQUESTS)
                .with_unit("{request}")
                .with_description("Number of active HTTP server requests")
                .build()
        });

        Self {
            instruments: HttpServerInstruments {
                request_duration,
                active_requests,
                request_body_size,
                response_body_size,
            },
        }
    }

    pub fn capture_request<'a>(
        &'a self,
        request: &HttpRequest,
    ) -> Capture<HttpServerRequestState<'a>> {
        if !self.instruments.is_enabled() {
            return Capture::disabled();
        }

        let method = request.method().as_static_str();
        let scheme = request.uri().scheme_static_str();

        Capture::enabled(HttpServerRequestState {
            instruments: &self.instruments,
            _active_request_guard: self.active_request_started(method, scheme),
            method,
            scheme,
            route: request.path().to_string(),
            protocol_version: request.version().as_static_str(),
            started_at: Instant::now(),
        })
    }

    fn active_request_started(
        &self,
        method: &'static str,
        scheme: &'static str,
    ) -> HttpServerActiveRequestGuard<'_> {
        if let Some(counter) = &self.instruments.active_requests {
            counter.add(1, &active_request_attributes(method, scheme));
        }

        HttpServerActiveRequestGuard {
            instruments: &self.instruments,
            method,
            scheme,
        }
    }
}

impl Capture<HttpServerRequestState<'_>> {
    pub fn finish(
        self,
        response: &HttpResponse,
        body_size: Option<u64>,
        graphql_operation_name: Option<&str>,
        graphql_operation_type: Option<&str>,
        graphql_response_status: values::GraphQLResponseStatus,
    ) {
        let Some(state) = self.take() else {
            return;
        };

        let mut attributes = vec![
            KeyValue::new(labels::HTTP_REQUEST_METHOD, state.method),
            KeyValue::new(
                labels::HTTP_RESPONSE_STATUS_CODE,
                i64::from(response.status().as_u16()),
            ),
            KeyValue::new(labels::HTTP_ROUTE, state.route),
            KeyValue::new(labels::NETWORK_PROTOCOL_NAME, "http"),
            KeyValue::new(labels::NETWORK_PROTOCOL_VERSION, state.protocol_version),
            KeyValue::new(labels::URL_SCHEME, state.scheme),
            KeyValue::new(
                labels::GRAPHQL_RESPONSE_STATUS,
                graphql_response_status.as_str(),
            ),
            KeyValue::new(
                labels::GRAPHQL_OPERATION_NAME,
                graphql_operation_name
                    .map(str::to_string)
                    .unwrap_or_else(|| values::UNKNOWN.to_string()),
            ),
        ];

        if let Some(graphql_operation_type) = graphql_operation_type {
            attributes.push(KeyValue::new(
                labels::GRAPHQL_OPERATION_TYPE,
                graphql_operation_type.to_string(),
            ));
        }

        let status_code = response.status().as_u16();
        if status_code >= 400 {
            attributes.push(KeyValue::new(labels::ERROR_TYPE, status_code as i64));
        }

        if let Some(histogram) = &state.instruments.request_duration {
            #[cfg(debug_assertions)]
            debug_assert_attrs(names::HTTP_SERVER_REQUEST_DURATION, &attributes);
            histogram.record(state.started_at.elapsed().as_secs_f64(), &attributes);
        }

        if let (Some(histogram), Some(body_size)) =
            (&state.instruments.request_body_size, body_size)
        {
            #[cfg(debug_assertions)]
            debug_assert_attrs(names::HTTP_SERVER_REQUEST_BODY_SIZE, &attributes);
            histogram.record(body_size, &attributes);
        }

        if let Some(histogram) = &state.instruments.response_body_size {
            if let Some(body_size) = response.body().as_ref().and_then(|body| match body.size() {
                BodySize::Sized(size) => Some(size),
                _ => None,
            }) {
                #[cfg(debug_assertions)]
                debug_assert_attrs(names::HTTP_SERVER_RESPONSE_BODY_SIZE, &attributes);
                histogram.record(body_size, &attributes);
            }
        }
    }
}

impl Drop for HttpServerActiveRequestGuard<'_> {
    fn drop(&mut self) {
        if let Some(counter) = &self.instruments.active_requests {
            counter.add(-1, &active_request_attributes(self.method, self.scheme));
        }
    }
}

fn active_request_attributes(method: &'static str, scheme: &'static str) -> [KeyValue; 3] {
    let attrs = [
        KeyValue::new(labels::HTTP_REQUEST_METHOD, method),
        KeyValue::new(labels::NETWORK_PROTOCOL_NAME, "http"),
        KeyValue::new(labels::URL_SCHEME, scheme),
    ];

    #[cfg(debug_assertions)]
    debug_assert_attrs(names::HTTP_SERVER_ACTIVE_REQUESTS, &attrs);

    attrs
}
