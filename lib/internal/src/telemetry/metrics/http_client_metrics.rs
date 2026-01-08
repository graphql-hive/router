//! https://opentelemetry.io/docs/specs/semconv/http/http-metrics
use std::time::Instant;

use opentelemetry::{metrics::Histogram, metrics::Meter, metrics::UpDownCounter, KeyValue};

use crate::http::{HttpMethodAsStr, HttpUriAsStr, HttpVersionAsStr};
use crate::telemetry::metrics::{labels, names, Capture};

struct HttpClientInstruments {
    request_duration: Option<Histogram<f64>>,
    active_requests: Option<UpDownCounter<i64>>,
    request_body_size: Option<Histogram<u64>>,
    response_body_size: Option<Histogram<u64>>,
}

impl HttpClientInstruments {
    fn is_enabled(&self) -> bool {
        self.request_duration.is_some()
            || self.active_requests.is_some()
            || self.request_body_size.is_some()
            || self.response_body_size.is_some()
    }
}

pub struct HttpClientRequestState<'a> {
    instruments: &'a HttpClientInstruments,
    _active_request_guard: HttpClientActiveRequestGuard<'a>,
    method: &'static str,
    server_address: String,
    server_port: u16,
    status_code: Option<u16>,
    scheme: &'static str,
    protocol_version: &'static str,
    started_at: Instant,
    request_body_size: u64,
}

pub struct HttpClientActiveRequestGuard<'a> {
    instruments: &'a HttpClientInstruments,
    method: &'static str,
    server_address: String,
    server_port: u16,
    scheme: &'static str,
}

pub struct HttpClientMetrics {
    instruments: HttpClientInstruments,
}

impl HttpClientMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let request_duration = meter.map(|meter| {
            meter
                .f64_histogram(names::HTTP_CLIENT_REQUEST_DURATION)
                .with_unit("s")
                .with_description("Duration of HTTP client requests")
                .build()
        });

        let request_body_size = meter.map(|meter| {
            meter
                .u64_histogram(names::HTTP_CLIENT_REQUEST_BODY_SIZE)
                .with_unit("By")
                .with_description("Size of HTTP client request bodies")
                .build()
        });

        let response_body_size = meter.map(|meter| {
            meter
                .u64_histogram(names::HTTP_CLIENT_RESPONSE_BODY_SIZE)
                .with_unit("By")
                .with_description("Size of HTTP client response bodies")
                .build()
        });

        let active_requests = meter.map(|meter| {
            meter
                .i64_up_down_counter(names::HTTP_CLIENT_ACTIVE_REQUESTS)
                .with_unit("{request}")
                .with_description("Number of active HTTP client requests")
                .build()
        });

        Self {
            instruments: HttpClientInstruments {
                request_duration,
                active_requests,
                request_body_size,
                response_body_size,
            },
        }
    }

    pub fn capture_request<'a, T>(
        &'a self,
        request: &http::Request<T>,
        body_size: u64,
    ) -> Capture<HttpClientRequestState<'a>> {
        if !self.instruments.is_enabled() {
            return Capture::disabled();
        }

        let method = request.method().as_static_str();
        let scheme = request.uri().scheme_static_str();
        let server_address = request.uri().host().unwrap_or("unknown").to_string();
        let server_port =
            request
                .uri()
                .port_u16()
                .unwrap_or_else(|| if scheme == "https" { 443 } else { 80 });
        let protocol_version = request.version().as_static_str();

        Capture::enabled(HttpClientRequestState {
            instruments: &self.instruments,
            _active_request_guard: self.active_request_started(
                method,
                server_address.clone(),
                server_port,
                scheme,
            ),
            method,
            server_address,
            server_port,
            scheme,
            protocol_version,
            started_at: Instant::now(),
            request_body_size: body_size,
            status_code: None,
        })
    }

    fn active_request_started(
        &self,
        method: &'static str,
        server_address: String,
        server_port: u16,
        scheme: &'static str,
    ) -> HttpClientActiveRequestGuard<'_> {
        if let Some(counter) = &self.instruments.active_requests {
            counter.add(
                1,
                &active_request_attributes(method, &server_address, server_port, scheme),
            );
        }

        HttpClientActiveRequestGuard {
            instruments: &self.instruments,
            method,
            server_address,
            server_port,
            scheme,
        }
    }
}

impl Capture<HttpClientRequestState<'_>> {
    pub fn finish_success(&self, response_body_size: u64) {
        self.record(Some(response_body_size), None);
    }

    pub fn finish_error(&self, error_type: &'static str) {
        self.record(None, Some(error_type));
    }

    pub fn set_status_code(&mut self, status_code: u16) {
        if let Some(state) = self.as_mut() {
            state.status_code = Some(status_code);
        }
    }

    fn record(&self, response_body_size: Option<u64>, error_type: Option<&str>) {
        let Some(state) = self.as_ref() else {
            return;
        };

        let mut attributes = vec![
            KeyValue::new(labels::HTTP_REQUEST_METHOD, state.method),
            KeyValue::new(labels::SERVER_ADDRESS, state.server_address.clone()),
            KeyValue::new(labels::SERVER_PORT, i64::from(state.server_port)),
            KeyValue::new(labels::NETWORK_PROTOCOL_NAME, "http"),
            KeyValue::new(labels::NETWORK_PROTOCOL_VERSION, state.protocol_version),
            KeyValue::new(labels::URL_SCHEME, state.scheme),
        ];

        if let Some(status_code) = state.status_code {
            attributes.push(KeyValue::new(
                labels::HTTP_RESPONSE_STATUS_CODE,
                status_code as i64,
            ));

            if status_code >= 400 {
                attributes.push(KeyValue::new(labels::ERROR_TYPE, status_code as i64));
            }
        }

        if let Some(error_type) = error_type {
            attributes.push(KeyValue::new(labels::ERROR_TYPE, error_type.to_string()));
        }

        if let Some(histogram) = &state.instruments.request_duration {
            histogram.record(state.started_at.elapsed().as_secs_f64(), &attributes);
        }

        if let Some(histogram) = &state.instruments.request_body_size {
            histogram.record(state.request_body_size, &attributes);
        }

        if let (Some(histogram), Some(body_size)) =
            (&state.instruments.response_body_size, response_body_size)
        {
            histogram.record(body_size, &attributes);
        }
    }
}

impl Drop for HttpClientActiveRequestGuard<'_> {
    fn drop(&mut self) {
        if let Some(counter) = &self.instruments.active_requests {
            counter.add(
                -1,
                &active_request_attributes(
                    self.method,
                    &self.server_address,
                    self.server_port,
                    self.scheme,
                ),
            );
        }
    }
}

fn active_request_attributes(
    method: &'static str,
    server_address: &str,
    server_port: u16,
    scheme: &'static str,
) -> [KeyValue; 4] {
    [
        KeyValue::new(labels::HTTP_REQUEST_METHOD, method),
        KeyValue::new(labels::SERVER_ADDRESS, server_address.to_string()),
        KeyValue::new(labels::SERVER_PORT, server_port as i64),
        KeyValue::new(labels::URL_SCHEME, scheme),
    ]
}
