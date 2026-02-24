use opentelemetry_proto::tonic::resource::v1::Resource;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};

use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::trace::v1::{Span, Status};
use prost::Message;
use tonic::{Request, Response, Status as TonicStatus};

pub use opentelemetry_proto::tonic::trace::v1::span::SpanKind;

/// Forbidden ports for OTLP HTTP and gRPC servers.
/// Those ports are reserved for other services used in e2e tests.
static FORBIDDEN_PORTS: &[u16] = &[80, 443, 8080, 8443, 4000, 4200, 3000];

#[derive(Debug)]
pub struct Baggage {
    pub map: HashMap<String, String>,
}

impl PartialEq for Baggage {
    fn eq(&self, other: &Self) -> bool {
        self.map == other.map
    }
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

impl<'a> TraceParent<'a> {
    /// Generates W3C Trace Context traceparent header value.
    /// Format: "00-{trace_id}-{span_id}-{trace_flags}"
    pub fn to_string(&self) -> String {
        let flags = if self.sampled { "01" } else { "00" };
        format!("00-{}-{}-{}", self.trace_id, self.span_id, flags)
    }

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
                .get("hive.kind")
                .map(|v| v == hive_kind)
                .unwrap_or(false)
        });

        found.unwrap_or_else(|| panic!("No span found with hive.kind = {}", hive_kind))
    }

    pub fn has_span_by_hive_kind(&self, hive_kind: &str) -> bool {
        self.spans
            .iter()
            .find(|span| {
                span.attributes
                    .get("hive.kind")
                    .map(|v| v == hive_kind)
                    .unwrap_or(false)
            })
            .is_some()
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

/// A simplified event representation
#[derive(Clone, Debug)]
pub struct CollectedEvent {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
}

/// Shared storage for traces and requests
/// Handles merging spans from multiple requests with the same trace_id
/// Preserves insertion order for both requests and traces
struct TraceStorage {
    requests: Arc<Mutex<Vec<OtlpRequest>>>,
    traces: Arc<Mutex<Vec<CollectedTrace>>>,
}

impl TraceStorage {
    fn new() -> Self {
        TraceStorage {
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

/// gRPC service implementation for OTLP trace collection
struct GrpcTraceCollector {
    storage: Arc<TraceStorage>,
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
    storage: Arc<TraceStorage>,
    _http_handle: Option<std::thread::JoinHandle<()>>,
    _grpc_handle: Option<tokio::task::JoinHandle<()>>,
    grpc_shutdown_tx: Option<oneshot::Sender<()>>,
    last_wait_for_traces_count: usize,
}

impl OtlpCollector {
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let storage = Arc::new(TraceStorage::new());

        let (http_address, http_handle) = Self::start_http_server(storage.clone()).await?;
        let (grpc_address, grpc_handle, grpc_shutdown_tx) =
            Self::start_grpc_server(storage.clone()).await?;

        println!("OTLP HTTP collector server started on {}", http_address);
        println!("OTLP gRPC collector server started on {}", grpc_address);

        Ok(OtlpCollector {
            http_address,
            grpc_address,
            storage,
            _http_handle: Some(http_handle),
            _grpc_handle: Some(grpc_handle),
            grpc_shutdown_tx: Some(grpc_shutdown_tx),
            last_wait_for_traces_count: 0,
        })
    }

    async fn start_http_server(
        storage: Arc<TraceStorage>,
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

        let storage_clone = storage.clone();
        let handle = std::thread::spawn(move || {
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

                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    // Try to decode and merge traces
                    if let Ok(export_req) = ExportTraceServiceRequest::decode(body.as_slice()) {
                        storage_clone.merge_export_request(&export_req).await;
                    }

                    storage_clone
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
        storage: Arc<TraceStorage>,
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

        let trace_service = GrpcTraceCollector { storage };

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

    pub fn http_endpoint(&self) -> String {
        self.http_address.clone()
    }

    pub fn grpc_endpoint(&self) -> String {
        self.grpc_address.clone()
    }

    pub async fn request_at(&self, request_idx: usize) -> Option<OtlpRequest> {
        self.storage.request_at(request_idx).await
    }

    pub async fn is_empty(&self) -> bool {
        self.storage.is_empty().await
    }

    pub async fn traces(&self) -> Vec<CollectedTrace> {
        self.storage.traces().await
    }

    /// Waits for new traces to arrive in storage, returning all collected traces. The waiting is
    /// done by checking the number of traces in storage and comparing it to the count from the last wait.
    pub async fn wait_for_traces(&mut self) -> Vec<CollectedTrace> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let traces = self.traces().await;

                if traces.len() > self.last_wait_for_traces_count {
                    // more traces came in since the last check
                    self.last_wait_for_traces_count = traces.len();
                    return traces;
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("waiting for traces timed out")
    }

    /// Waits for at least `count` traces to be collected, returning all collected traces.
    pub async fn wait_for_traces_count(&self, count: usize) -> Vec<CollectedTrace> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let traces = self.traces().await;

                if traces.len() >= count {
                    return traces;
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("waiting for traces with count timed out")
    }

    pub async fn wait_for_span_by_hive_kind_one(&self, hive_kind: &str) -> CollectedSpan {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let traces = self.traces().await;

                let found = traces.into_iter().find_map(|trace| {
                    trace.spans.into_iter().find(|span| {
                        span.attributes
                            .get("hive.kind")
                            .map(|v| v == hive_kind)
                            .unwrap_or(false)
                    })
                });
                if let Some(span) = found {
                    return span;
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("waiting for span by hive kind timed out")
    }

    /// Common insta filter settings for OTLP-related snapshots that filters
    /// out unstable/dynamic values like ports and random keys.
    //
    // P.S. its a method only because of convenience not having to import another module
    // or function from this file and instead just do `otel_collector.insta_filter_settings()`.
    pub fn insta_filter_settings(&self) -> insta::Settings {
        let mut settings = insta::Settings::new();

        // keys
        settings.add_filter(r"(hive\.inflight\.key:\s+)\d+", "$1[random]");

        // addresses and ports
        settings.add_filter(r"(server\.address:\s+)[\d.]+", "$1[address]");
        settings.add_filter(r"(server\.port:\s+)\d+", "$1[port]");
        settings.add_filter(
            r"(url\.full:\s+http:\/\/)[\d.]+:\d+(.*)",
            "$1[address]:[port]$2",
        );
        settings.add_filter(
            r"(http\.url:\s+http:\/\/)[\d.]+:\d+(.*)",
            "$1[address]:[port]$2",
        );
        settings.add_filter(r"(net\.peer\.name:\s+)[\d.]+", "$1[address]");
        settings.add_filter(r"(net\.peer\.port:\s+)\d+", "$1[port]");

        settings
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
    let mut spans = Vec::new();

    for resource_span in &request.resource_spans {
        for scope_span in &resource_span.scope_spans {
            for span in &scope_span.spans {
                spans.push(span.clone());
            }
        }
    }

    spans
}

fn extract_resource(request: &ExportTraceServiceRequest) -> Vec<Resource> {
    let mut resources = Vec::new();

    for resource_span in &request.resource_spans {
        if let Some(resource) = &resource_span.resource {
            resources.push(resource.clone());
        }
    }

    resources
}

impl From<&Span> for CollectedSpan {
    fn from(span: &Span) -> Self {
        let mut attributes = BTreeMap::new();
        for kv in &span.attributes {
            let key = &kv.key;
            if let Some(value) = &kv.value {
                attributes.insert(key.clone(), format_attribute_value(value));
            }
        }

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
        let mut attributes = BTreeMap::new();
        for kv in &resource.attributes {
            let key = &kv.key;
            if let Some(value) = &kv.value {
                attributes.insert(key.clone(), format_attribute_value(value));
            }
        }

        Self { attributes }
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
            let values: Vec<String> = arr
                .values
                .iter()
                .map(|v| format_attribute_value(v))
                .collect();
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
