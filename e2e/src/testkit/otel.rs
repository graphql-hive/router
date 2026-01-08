use hive_router_internal::telemetry::traces::spans::attributes::HIVE_KIND;
use opentelemetry_proto::tonic::resource::v1::Resource;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Display;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::KeyValue;
use opentelemetry_proto::tonic::metrics::v1::{
    metric::Data as MetricData, number_data_point::Value as NumberDataPointValue, NumberDataPoint,
};
use opentelemetry_proto::tonic::trace::v1::{Span, Status};
use prost::Message;
use tonic::{Request, Response, Status as TonicStatus};

pub use opentelemetry_proto::tonic::trace::v1::span::SpanKind;

/// Forbidden ports for OTLP HTTP and gRPC servers.
/// Those ports are reserved for other services used in e2e tests.
static FORBIDDEN_PORTS: &[u16] = &[80, 443, 8080, 8443, 4000, 4200, 3000];

#[derive(Debug, PartialEq)]
pub struct Baggage {
    pub map: HashMap<String, String>,
}

impl<const N: usize> From<[(String, String); N]> for Baggage {
    fn from(arr: [(String, String); N]) -> Self {
        Self {
            map: HashMap::from(arr),
        }
    }
}

impl From<&str> for Baggage {
    fn from(value: &str) -> Self {
        Self {
            map: value
                .split(',')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}

impl Display for Baggage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut entries = self
            .map
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>();
        entries.sort();
        write!(f, "{}", entries.join(","))
    }
}

pub struct TraceParent<'a> {
    pub trace_id: &'a str,
    pub span_id: &'a str,
    pub sampled: bool,
}

impl Display for TraceParent<'_> {
    /// Generates W3C Trace Context traceparent header value.
    /// Format: "00-{trace_id}-{span_id}-{trace_flags}"
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let flags = if self.sampled { "01" } else { "00" };
        write!(f, "00-{}-{}-{}", self.trace_id, self.span_id, flags)
    }
}

impl<'a> TraceParent<'a> {
    pub fn parse(traceparent: &'a str) -> Self {
        let parts: Vec<&str> = traceparent.split('-').collect();
        assert_eq!(parts.len(), 4, "traceparent should have 4 parts");

        Self {
            trace_id: parts[1],
            span_id: parts[2],
            sampled: parts[3] == "01",
        }
    }

    pub fn random_trace_id() -> String {
        let random: u128 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{:032x}", random)
    }

    pub fn random_span_id() -> String {
        let random: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        format!("{:016x}", random)
    }
}

/// Represents a single OTLP request captured by the collector
#[derive(Debug, Clone)]
pub struct OtlpRequest {
    #[allow(dead_code)]
    pub method: String,
    #[allow(dead_code)]
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct CollectedTrace {
    pub id: String,
    pub spans: Vec<CollectedSpan>,
    pub resources: Vec<CollectedResource>,
    pub events: Vec<CollectedEvent>,
}

impl CollectedTrace {
    pub fn new(id: String) -> Self {
        CollectedTrace {
            id,
            spans: Vec::new(),
            resources: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn add_span(&mut self, span: CollectedSpan) {
        self.spans.push(span);
    }

    pub fn add_resource(&mut self, resource: CollectedResource) {
        self.resources.push(resource);
    }

    pub fn add_event(&mut self, event: CollectedEvent) {
        self.events.push(event);
    }

    pub fn merged_resource_attributes(&self) -> BTreeMap<String, String> {
        let mut merged_attributes = BTreeMap::new();
        for resource in &self.resources {
            for (key, value) in resource.attributes.iter() {
                merged_attributes.insert(key.clone(), value.clone());
            }
        }
        merged_attributes
    }

    /// Find a span by hive.kind attribute value.
    /// Panics if not found!
    pub fn span_by_hive_kind_one(&self, hive_kind: &str) -> &CollectedSpan {
        let found = self.spans.iter().find(|span| {
            span.attributes
                .get(HIVE_KIND)
                .map(|v| v == hive_kind)
                .unwrap_or(false)
        });

        found.unwrap_or_else(|| panic!("No span found with hive.kind = {}", hive_kind))
    }

    pub fn has_span_by_hive_kind(&self, hive_kind: &str) -> bool {
        self.spans.iter().any(|span| {
            span.attributes
                .get(HIVE_KIND)
                .map(|v| v == hive_kind)
                .unwrap_or(false)
        })
    }
}

#[derive(Clone, Debug)]
pub struct CollectedSpan {
    pub id: String,
    pub parent_span_id: String,
    pub trace_id: String,
    pub name: String,
    pub kind: Option<SpanKind>,
    pub status: Option<Status>,
    pub attributes: BTreeMap<String, String>,
    pub events: Vec<CollectedEvent>,
}

#[derive(Clone, Debug)]
pub struct CollectedResource {
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct CollectedMetric {
    pub name: String,
    pub resource_attributes: BTreeMap<String, String>,
    pub data: CollectedMetricData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NumberMetricKind {
    Counter,
    Gauge,
}

#[derive(Clone, Debug)]
pub enum CollectedMetricData {
    Number {
        kind: NumberMetricKind,
        points: Vec<CollectedMetricDataPoint>,
    },
    Histogram {
        points: Vec<CollectedHistogramPoint>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SeriesKind {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Clone, Debug)]
pub struct CollectedMetricDataPoint {
    pub attributes: BTreeMap<String, String>,
    pub value: f64,
    pub start_time_unix_nano: u64,
    pub time_unix_nano: u64,
}

#[derive(Clone, Debug)]
pub struct CollectedHistogramPoint {
    pub attributes: BTreeMap<String, String>,
    pub count: u64,
    pub sum: f64,
    pub start_time_unix_nano: u64,
    pub time_unix_nano: u64,
}

#[derive(Clone, Debug)]
pub struct CollectedMetrics {
    latest_by_series: BTreeMap<SeriesKey, LatestSeriesPoint>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SeriesKey {
    name: String,
    kind: SeriesKind,
    attrs: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
enum LatestSeriesPoint {
    Number {
        value: f64,
        start_time_unix_nano: u64,
        time_unix_nano: u64,
    },
    Histogram {
        count: u64,
        sum: f64,
        start_time_unix_nano: u64,
        time_unix_nano: u64,
    },
}

impl CollectedMetrics {
    pub fn new(metrics: Vec<CollectedMetric>) -> Self {
        let latest_by_series = build_latest_series_index(&metrics);
        Self { latest_by_series }
    }

    pub fn latest_counter(&self, name: &str, attrs: &[(&str, &str)]) -> f64 {
        self.latest_number(name, SeriesKind::Counter, attrs)
    }

    pub fn has_counter(&self, name: &str, attrs: &[(&str, &str)]) -> bool {
        self.has_series(name, SeriesKind::Counter, attrs)
    }

    pub fn has_gauge(&self, name: &str, attrs: &[(&str, &str)]) -> bool {
        self.has_series(name, SeriesKind::Gauge, attrs)
    }

    pub fn has_histogram(&self, name: &str, attrs: &[(&str, &str)]) -> bool {
        self.has_series(name, SeriesKind::Histogram, attrs)
    }

    pub fn latest_histogram_count_sum(&self, name: &str, attrs: &[(&str, &str)]) -> (u64, f64) {
        self.latest_by_series
            .iter()
            .filter_map(|(key, point)| {
                if key.name != name
                    || key.kind != SeriesKind::Histogram
                    || !attributes_match_subset(&key.attrs, attrs)
                {
                    return None;
                }

                match point {
                    LatestSeriesPoint::Histogram { count, sum, .. } => Some((*count, *sum)),
                    LatestSeriesPoint::Number { .. } => None,
                }
            })
            .fold((0, 0.0), |(acc_count, acc_sum), (count, sum)| {
                (acc_count + count, acc_sum + sum)
            })
    }

    fn latest_number(&self, name: &str, kind: SeriesKind, attrs: &[(&str, &str)]) -> f64 {
        self.latest_by_series
            .iter()
            .filter_map(|(key, point)| {
                if key.name != name
                    || key.kind != kind
                    || !attributes_match_subset(&key.attrs, attrs)
                {
                    return None;
                }

                match point {
                    LatestSeriesPoint::Number { value, .. } => Some(*value),
                    LatestSeriesPoint::Histogram { .. } => None,
                }
            })
            .sum()
    }

    fn has_series(&self, name: &str, kind: SeriesKind, attrs: &[(&str, &str)]) -> bool {
        self.latest_by_series.iter().any(|(key, _)| {
            key.name == name && key.kind == kind && attributes_match_subset(&key.attrs, attrs)
        })
    }

    pub fn latest_attribute_names(&self, name: &str) -> BTreeSet<String> {
        self.latest_by_series
            .iter()
            .filter(|(key, _)| key.name == name)
            .flat_map(|(key, _)| key.attrs.keys().cloned())
            .collect()
    }
}

/// A simplified event representation
#[derive(Clone, Debug)]
pub struct CollectedEvent {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
}

/// Shared storage for traces and requests
/// Handles merging spans from multiple requests with the same trace_id
/// Preserves insertion order for both requests and traces
struct TracesStorage {
    requests: Arc<Mutex<Vec<OtlpRequest>>>,
    traces: Arc<Mutex<Vec<CollectedTrace>>>,
}

impl TracesStorage {
    fn new() -> Self {
        TracesStorage {
            requests: Arc::new(Mutex::new(Vec::new())),
            traces: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn add_request(&self, request: OtlpRequest) {
        self.requests.lock().await.push(request);
    }

    /// Merge spans and resources from an export request into traces
    async fn merge_export_request(&self, export_req: &ExportTraceServiceRequest) {
        let spans = extract_spans(export_req);
        let resources = extract_resource(export_req);

        let mut traces = self.traces.lock().await;

        for span in &spans {
            let trace_id = extract_trace_id(span);
            let collected_span = CollectedSpan::from(span);
            let events = collected_span.events.clone();

            // Find or create trace with this ID, preserving insertion order
            if let Some(trace) = traces.iter_mut().find(|t| t.id == trace_id) {
                trace.add_span(collected_span);
                // Add events from span to trace
                for event in events {
                    trace.add_event(event);
                }
            } else {
                let mut new_trace = CollectedTrace::new(trace_id);
                new_trace.add_span(collected_span);
                // Add events from span to trace
                for event in events {
                    new_trace.add_event(event);
                }
                traces.push(new_trace);
            }
        }

        // Add resources to all affected traces
        for resource in resources {
            let collected_resource = CollectedResource::from(&resource);
            for span in &spans {
                let trace_id = extract_trace_id(span);
                if let Some(trace) = traces.iter_mut().find(|t| t.id == trace_id) {
                    trace.add_resource(collected_resource.clone());
                }
            }
        }
    }

    async fn traces(&self) -> Vec<CollectedTrace> {
        self.traces.lock().await.clone()
    }

    async fn request_at(&self, idx: usize) -> Option<OtlpRequest> {
        self.requests.lock().await.get(idx).cloned()
    }

    async fn is_empty(&self) -> bool {
        self.requests.lock().await.iter().all(|f| f.body.is_empty())
    }
}

struct MetricsStorage {
    metrics: Arc<Mutex<Vec<CollectedMetric>>>,
}

impl MetricsStorage {
    fn new() -> Self {
        MetricsStorage {
            metrics: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn merge_export_request(&self, export_req: &ExportMetricsServiceRequest) {
        let metrics = extract_metrics(export_req);
        self.metrics.lock().await.extend(metrics);
    }

    async fn metrics(&self) -> Vec<CollectedMetric> {
        self.metrics.lock().await.clone()
    }
}

/// gRPC service implementation for OTLP trace collection
struct GrpcTraceCollector {
    storage: Arc<TracesStorage>,
}

#[tonic::async_trait]
impl TraceService for GrpcTraceCollector {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, TonicStatus> {
        let headers: Vec<(String, String)> = request
            .metadata()
            .iter()
            .filter_map(|item| match item {
                tonic::metadata::KeyAndValueRef::Ascii(key, value) => {
                    let key = key.as_str().to_string();
                    let value = value.to_str().ok()?.to_string();
                    Some((key, value))
                }
                tonic::metadata::KeyAndValueRef::Binary(_key, _value) => None,
            })
            .collect();

        let req = request.into_inner();

        // Encode the request back to bytes for storage
        let body = req.encode_to_vec();

        println!("Captured gRPC OTLP export request");

        self.storage.merge_export_request(&req).await;

        // Store request
        let otlp_req = OtlpRequest {
            method: "POST".to_string(),
            path: "/opentelemetry.proto.collector.trace.v1.TraceService/Export".to_string(),
            headers,
            body,
        };

        self.storage.add_request(otlp_req).await;

        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: None,
        }))
    }
}

/// Mock OTLP collector server that captures incoming trace export requests
pub struct OtlpCollector {
    pub http_address: String,
    pub grpc_address: String,
    traces_storage: Arc<TracesStorage>,
    metrics_storage: Arc<MetricsStorage>,
    _http_handle: Option<std::thread::JoinHandle<()>>,
    _grpc_handle: Option<tokio::task::JoinHandle<()>>,
    grpc_shutdown_tx: Option<oneshot::Sender<()>>,
}

impl OtlpCollector {
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let traces_storage = Arc::new(TracesStorage::new());
        let metrics_storage = Arc::new(MetricsStorage::new());

        let (http_address, http_handle) =
            Self::start_http_server(traces_storage.clone(), metrics_storage.clone()).await?;
        let (grpc_address, grpc_handle, grpc_shutdown_tx) =
            Self::start_grpc_server(traces_storage.clone()).await?;

        println!("OTLP HTTP collector server started on {}", http_address);
        println!("OTLP gRPC collector server started on {}", grpc_address);

        Ok(OtlpCollector {
            http_address,
            grpc_address,
            traces_storage,
            metrics_storage,
            _http_handle: Some(http_handle),
            _grpc_handle: Some(grpc_handle),
            grpc_shutdown_tx: Some(grpc_shutdown_tx),
        })
    }

    async fn start_http_server(
        traces_storage: Arc<TracesStorage>,
        metrics_storage: Arc<MetricsStorage>,
    ) -> Result<(String, std::thread::JoinHandle<()>), Box<dyn std::error::Error>> {
        // Binding to port 0 tell the OS to assign a random available port.
        let server = loop {
            let server = tiny_http::Server::http("127.0.0.1:0")
                .map_err(|e| format!("Failed to start OTLP HTTP server: {}", e))?;

            let port = server
                .server_addr()
                .to_string()
                .parse::<std::net::SocketAddr>()
                .expect("Failed to parse socket address")
                .port();

            if !FORBIDDEN_PORTS.contains(&port) {
                break server;
            }
        };
        let address_str = format!("http://{}", server.server_addr());

        let traces_storage_clone = traces_storage.clone();
        let metrics_storage_clone = metrics_storage.clone();
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for OTLP HTTP collector");

            for mut request in server.incoming_requests() {
                let method = request.method().to_string();
                let path = request.url().to_string();

                let headers: Vec<(String, String)> = request
                    .headers()
                    .iter()
                    .map(|h| (h.field.to_string(), h.value.to_string()))
                    .collect();

                let mut body = Vec::new();
                let _ = request.as_reader().read_to_end(&mut body);

                println!("Captured OTLP HTTP request: {} {}", method, path);

                rt.block_on(async {
                    if path.ends_with("/v1/traces") {
                        if let Ok(export_req) = ExportTraceServiceRequest::decode(body.as_slice()) {
                            traces_storage_clone.merge_export_request(&export_req).await;
                        }
                    }
                    if path.ends_with("/v1/metrics") {
                        if let Ok(export_req) = ExportMetricsServiceRequest::decode(body.as_slice())
                        {
                            metrics_storage_clone
                                .merge_export_request(&export_req)
                                .await;
                        }
                    }

                    traces_storage_clone
                        .add_request(OtlpRequest {
                            method,
                            path,
                            headers,
                            body,
                        })
                        .await;
                });

                // Without the response, the exporter will log an error saying the request failed to respond
                let _ = request.respond(tiny_http::Response::from_string(""));
            }
        });

        Ok((address_str, handle))
    }

    async fn start_grpc_server(
        traces_storage: Arc<TracesStorage>,
    ) -> Result<
        (String, tokio::task::JoinHandle<()>, oneshot::Sender<()>),
        Box<dyn std::error::Error>,
    > {
        // Binding to port 0 tell the OS to assign a random available port.
        let (listener, addr) = loop {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let addr = listener.local_addr()?;
            if !FORBIDDEN_PORTS.contains(&addr.port()) {
                break (listener, addr);
            }
        };
        let grpc_address = format!("http://{}", addr);

        let trace_service = GrpcTraceCollector {
            storage: traces_storage,
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        // Spawn gRPC server
        let handle = tokio::spawn(async move {
            let result = tonic::transport::Server::builder()
                .add_service(TraceServiceServer::new(trace_service))
                .serve_with_incoming_shutdown(
                    tokio_stream::wrappers::TcpListenerStream::new(listener),
                    async {
                        shutdown_rx.await.ok();
                    },
                )
                .await;

            if let Err(e) = result {
                eprintln!("gRPC server error: {}", e);
            }
        });

        Ok((grpc_address, handle, shutdown_tx))
    }

    pub fn http_traces_endpoint(&self) -> String {
        format!("{}/v1/traces", self.http_address)
    }

    pub fn http_metrics_endpoint(&self) -> String {
        format!("{}/v1/metrics", self.http_address)
    }

    pub fn grpc_endpoint(&self) -> String {
        self.grpc_address.clone()
    }

    pub async fn request_at(&self, request_idx: usize) -> Option<OtlpRequest> {
        self.traces_storage.request_at(request_idx).await
    }

    pub async fn is_empty(&self) -> bool {
        self.traces_storage.is_empty().await
    }

    pub async fn traces(&self) -> Vec<CollectedTrace> {
        self.traces_storage.traces().await
    }

    pub async fn metrics_view(&self) -> CollectedMetrics {
        CollectedMetrics::new(self.metrics_storage.metrics().await)
    }
}

impl Drop for OtlpCollector {
    fn drop(&mut self) {
        // Trigger graceful shutdown of the gRPC server
        if let Some(shutdown_tx) = self.grpc_shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

/// Extract trace_id from a span, converting bytes to hex string
fn extract_trace_id(span: &Span) -> String {
    hex::encode(&span.trace_id)
}

fn extract_spans(request: &ExportTraceServiceRequest) -> Vec<Span> {
    request
        .resource_spans
        .iter()
        .flat_map(|resource_span| resource_span.scope_spans.iter())
        .flat_map(|scope_span| scope_span.spans.iter().cloned())
        .collect()
}

fn extract_resource(request: &ExportTraceServiceRequest) -> Vec<Resource> {
    request
        .resource_spans
        .iter()
        .filter_map(|resource_span| resource_span.resource.clone())
        .collect()
}

fn extract_metrics(request: &ExportMetricsServiceRequest) -> Vec<CollectedMetric> {
    let mut metrics = Vec::new();

    for resource_metrics in &request.resource_metrics {
        let resource_attributes = resource_metrics
            .resource
            .as_ref()
            .map(|resource| CollectedResource::from(resource).attributes)
            .unwrap_or_default();

        for scope_metrics in &resource_metrics.scope_metrics {
            for metric in &scope_metrics.metrics {
                if let Some(data) = &metric.data {
                    match data {
                        MetricData::Sum(sum) => collect_number_metrics(
                            &metric.name,
                            &resource_attributes,
                            NumberMetricKind::Counter,
                            &sum.data_points,
                            &mut metrics,
                        ),
                        MetricData::Gauge(gauge) => collect_number_metrics(
                            &metric.name,
                            &resource_attributes,
                            NumberMetricKind::Gauge,
                            &gauge.data_points,
                            &mut metrics,
                        ),
                        MetricData::Histogram(histogram) => collect_histogram_metrics(
                            &metric.name,
                            &resource_attributes,
                            &histogram.data_points,
                            &mut metrics,
                        ),
                        MetricData::ExponentialHistogram(histogram) => {
                            collect_exponential_histogram_metrics(
                                &metric.name,
                                &resource_attributes,
                                &histogram.data_points,
                                &mut metrics,
                            )
                        }
                        MetricData::Summary(_) => {}
                    }
                }
            }
        }
    }

    metrics
}

fn collect_number_metrics(
    name: &str,
    resource_attributes: &BTreeMap<String, String>,
    kind: NumberMetricKind,
    data_points: &[NumberDataPoint],
    metrics: &mut Vec<CollectedMetric>,
) {
    let mut number_data_points = Vec::new();
    for point in data_points {
        if let Some(value) = number_data_point_value(point) {
            number_data_points.push(CollectedMetricDataPoint {
                attributes: attributes_from_kvlist(&point.attributes),
                value,
                start_time_unix_nano: point.start_time_unix_nano,
                time_unix_nano: point.time_unix_nano,
            });
        }
    }

    if number_data_points.is_empty() {
        return;
    }

    metrics.push(CollectedMetric {
        name: name.to_string(),
        resource_attributes: resource_attributes.clone(),
        data: CollectedMetricData::Number {
            kind,
            points: number_data_points,
        },
    });
}

fn collect_histogram_metrics(
    name: &str,
    resource_attributes: &BTreeMap<String, String>,
    data_points: &[opentelemetry_proto::tonic::metrics::v1::HistogramDataPoint],
    metrics: &mut Vec<CollectedMetric>,
) {
    let mut histogram_data_points = Vec::new();
    for point in data_points {
        let sum = point.sum.unwrap_or_default();
        histogram_data_points.push(CollectedHistogramPoint {
            attributes: attributes_from_kvlist(&point.attributes),
            count: point.count,
            sum,
            start_time_unix_nano: point.start_time_unix_nano,
            time_unix_nano: point.time_unix_nano,
        });
    }

    if histogram_data_points.is_empty() {
        return;
    }

    metrics.push(CollectedMetric {
        name: name.to_string(),
        resource_attributes: resource_attributes.clone(),
        data: CollectedMetricData::Histogram {
            points: histogram_data_points,
        },
    });
}

fn collect_exponential_histogram_metrics(
    name: &str,
    resource_attributes: &BTreeMap<String, String>,
    data_points: &[opentelemetry_proto::tonic::metrics::v1::ExponentialHistogramDataPoint],
    metrics: &mut Vec<CollectedMetric>,
) {
    let mut histogram_data_points = Vec::new();
    for point in data_points {
        let sum = point.sum.unwrap_or_default();
        histogram_data_points.push(CollectedHistogramPoint {
            attributes: attributes_from_kvlist(&point.attributes),
            count: point.count,
            sum,
            start_time_unix_nano: point.start_time_unix_nano,
            time_unix_nano: point.time_unix_nano,
        });
    }

    if histogram_data_points.is_empty() {
        return;
    }

    metrics.push(CollectedMetric {
        name: name.to_string(),
        resource_attributes: resource_attributes.clone(),
        data: CollectedMetricData::Histogram {
            points: histogram_data_points,
        },
    });
}

fn number_data_point_value(point: &NumberDataPoint) -> Option<f64> {
    match point.value {
        Some(NumberDataPointValue::AsDouble(value)) => Some(value),
        Some(NumberDataPointValue::AsInt(value)) => Some(value as f64),
        None => None,
    }
}

fn attributes_from_kvlist(attributes: &[KeyValue]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for kv in attributes {
        let key = &kv.key;
        if let Some(value) = &kv.value {
            map.insert(key.clone(), format_attribute_value(value));
        }
    }
    map
}

fn merged_attributes(
    resource_attributes: &BTreeMap<String, String>,
    point_attributes: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = resource_attributes.clone();
    for (key, value) in point_attributes {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

fn attributes_match_subset(attributes: &BTreeMap<String, String>, subset: &[(&str, &str)]) -> bool {
    subset
        .iter()
        .all(|(key, value)| attributes.get(*key).map(String::as_str) == Some(*value))
}

fn should_replace_latest(
    current: &LatestSeriesPoint,
    incoming_time: u64,
    incoming_start: u64,
) -> bool {
    let (current_time, current_start) = match current {
        LatestSeriesPoint::Number {
            time_unix_nano,
            start_time_unix_nano,
            ..
        }
        | LatestSeriesPoint::Histogram {
            time_unix_nano,
            start_time_unix_nano,
            ..
        } => (*time_unix_nano, *start_time_unix_nano),
    };

    incoming_time > current_time
        || (incoming_time == current_time && incoming_start > current_start)
}

fn build_latest_series_index(
    metrics: &[CollectedMetric],
) -> BTreeMap<SeriesKey, LatestSeriesPoint> {
    use std::collections::btree_map::Entry;

    let mut index = BTreeMap::new();

    for metric in metrics {
        match &metric.data {
            CollectedMetricData::Number { kind, points } => {
                let series_kind = match kind {
                    NumberMetricKind::Counter => SeriesKind::Counter,
                    NumberMetricKind::Gauge => SeriesKind::Gauge,
                };

                for point in points {
                    let key = SeriesKey {
                        name: metric.name.clone(),
                        kind: series_kind.clone(),
                        attrs: merged_attributes(&metric.resource_attributes, &point.attributes),
                    };

                    let incoming = LatestSeriesPoint::Number {
                        value: point.value,
                        start_time_unix_nano: point.start_time_unix_nano,
                        time_unix_nano: point.time_unix_nano,
                    };

                    match index.entry(key) {
                        Entry::Vacant(entry) => {
                            entry.insert(incoming);
                        }
                        Entry::Occupied(mut entry)
                            if should_replace_latest(
                                entry.get(),
                                point.time_unix_nano,
                                point.start_time_unix_nano,
                            ) =>
                        {
                            entry.insert(incoming);
                        }
                        Entry::Occupied(_) => {}
                    }
                }
            }
            CollectedMetricData::Histogram { points } => {
                for point in points {
                    let key = SeriesKey {
                        name: metric.name.clone(),
                        kind: SeriesKind::Histogram,
                        attrs: merged_attributes(&metric.resource_attributes, &point.attributes),
                    };

                    let incoming = LatestSeriesPoint::Histogram {
                        count: point.count,
                        sum: point.sum,
                        start_time_unix_nano: point.start_time_unix_nano,
                        time_unix_nano: point.time_unix_nano,
                    };

                    match index.entry(key) {
                        Entry::Vacant(entry) => {
                            entry.insert(incoming);
                        }
                        Entry::Occupied(mut entry)
                            if should_replace_latest(
                                entry.get(),
                                point.time_unix_nano,
                                point.start_time_unix_nano,
                            ) =>
                        {
                            entry.insert(incoming);
                        }
                        Entry::Occupied(_) => {}
                    }
                }
            }
        }
    }

    index
}

impl From<&Span> for CollectedSpan {
    fn from(span: &Span) -> Self {
        let attributes = attributes_from_kvlist(&span.attributes);

        let events = span
            .events
            .iter()
            .map(|event| CollectedEvent {
                name: event.name.clone(),
                attributes: event
                    .attributes
                    .iter()
                    .map(|kv| {
                        if let Some(value) = &kv.value {
                            (kv.key.clone(), format_attribute_value(value))
                        } else {
                            (kv.key.clone(), "null".to_string())
                        }
                    })
                    .collect(),
            })
            .collect();

        CollectedSpan {
            id: hex::encode(&span.span_id),
            parent_span_id: hex::encode(&span.parent_span_id),
            trace_id: hex::encode(&span.trace_id),
            name: span.name.clone(),
            kind: format_span_kind(span.kind),
            status: span.status.clone(),
            attributes,
            events,
        }
    }
}

impl From<&Resource> for CollectedResource {
    fn from(resource: &Resource) -> Self {
        Self {
            attributes: attributes_from_kvlist(&resource.attributes),
        }
    }
}

fn format_span_kind(kind: i32) -> Option<SpanKind> {
    SpanKind::try_from(kind).ok()
}

fn format_attribute_value(value: &opentelemetry_proto::tonic::common::v1::AnyValue) -> String {
    match &value.value {
        Some(Value::StringValue(s)) => s.clone(),
        Some(Value::IntValue(i)) => i.to_string(),
        Some(Value::DoubleValue(d)) => d.to_string(),
        Some(Value::BoolValue(b)) => b.to_string(),
        Some(Value::ArrayValue(arr)) => {
            let values: Vec<String> = arr.values.iter().map(format_attribute_value).collect();
            format!("[{}]", values.join(", "))
        }
        Some(Value::KvlistValue(kvlist)) => {
            let pairs: Vec<String> = kvlist
                .values
                .iter()
                .map(|kv| {
                    if let Some(value) = &kv.value {
                        format!("{}={}", kv.key, format_attribute_value(value))
                    } else {
                        format!("{}=null", kv.key)
                    }
                })
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        Some(Value::BytesValue(b)) => hex::encode(b),
        None => "null".to_string(),
    }
}

impl Display for CollectedSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Span: {}", self.name)?;

        match &self.kind {
            Some(kind) => {
                writeln!(f, "  Kind: {:?}", kind)?;
            }
            None => {
                writeln!(f, "  Kind: None")?;
            }
        }

        match &self.status {
            Some(status) => {
                writeln!(
                    f,
                    "  Status: message='{}' code='{}'",
                    &status.message, &status.code,
                )?;
            }
            None => {
                writeln!(f, "  Status: None")?;
            }
        }

        if !self.attributes.is_empty() {
            writeln!(f, "  Attributes:")?;
            for (key, value) in &self.attributes {
                writeln!(f, "    {}: {}", key, value)?;
            }
        }

        if !self.events.is_empty() {
            writeln!(f, "  Events:")?;
            for event in &self.events {
                writeln!(f, "    - {}", event.name)?;
                if !event.attributes.is_empty() {
                    for (key, value) in &event.attributes {
                        writeln!(f, "      {}: {}", key, value)?;
                    }
                }
            }
        }

        Ok(())
    }
}
