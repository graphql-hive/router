use opentelemetry_proto::tonic::resource::v1::Resource;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone)]
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

#[derive(Clone)]
pub struct CollectedResource {
    pub attributes: BTreeMap<String, String>,
}

/// A simplified event representation
#[derive(Clone)]
pub struct CollectedEvent {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
}

/// gRPC service implementation for OTLP trace collection
struct GrpcTraceCollector {
    requests: Arc<Mutex<Vec<OtlpRequest>>>,
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

        // Store request
        let otlp_req = OtlpRequest {
            method: "POST".to_string(),
            path: "/opentelemetry.proto.collector.trace.v1.TraceService/Export".to_string(),
            headers,
            body,
        };

        self.requests.lock().await.push(otlp_req);

        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: None,
        }))
    }
}

/// Mock OTLP collector server that captures incoming trace export requests
pub struct OtlpCollector {
    pub http_address: String,
    pub grpc_address: String,
    requests: Arc<Mutex<Vec<OtlpRequest>>>,
    _http_handle: Option<std::thread::JoinHandle<()>>,
    _grpc_handle: Option<tokio::task::JoinHandle<()>>,
}

impl OtlpCollector {
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let requests = Arc::new(Mutex::new(Vec::new()));

        let (http_address, http_handle) = Self::start_http_server(requests.clone()).await?;
        let (grpc_address, grpc_handle) = Self::start_grpc_server(requests.clone()).await?;

        println!("OTLP HTTP collector server started on {}", http_address);
        println!("OTLP gRPC collector server started on {}", grpc_address);

        Ok(OtlpCollector {
            http_address,
            grpc_address,
            requests,
            _http_handle: Some(http_handle),
            _grpc_handle: Some(grpc_handle),
        })
    }

    async fn start_http_server(
        requests: Arc<Mutex<Vec<OtlpRequest>>>,
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

        let requests_clone = requests.clone();
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
                    requests_clone.lock().await.push(OtlpRequest {
                        method,
                        path,
                        headers,
                        body,
                    });
                });

                // Without the response, the exporter will log an error saying the request failed to respond
                let _ = request.respond(tiny_http::Response::from_string(""));
            }
        });

        Ok((address_str, handle))
    }

    async fn start_grpc_server(
        requests: Arc<Mutex<Vec<OtlpRequest>>>,
    ) -> Result<(String, tokio::task::JoinHandle<()>), Box<dyn std::error::Error>> {
        // Binding to port 0 tell the OS to assign a random available port.
        let (listener, addr) = loop {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let addr = listener.local_addr()?;
            if !FORBIDDEN_PORTS.contains(&addr.port()) {
                break (listener, addr);
            }
        };
        let grpc_address = format!("http://{}", addr);

        let requests_clone = requests.clone();

        let trace_service = GrpcTraceCollector {
            requests: requests_clone,
        };

        // Spawn gRPC server
        let handle = tokio::spawn(async move {
            let result = tonic::transport::Server::builder()
                .add_service(TraceServiceServer::new(trace_service))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await;

            if let Err(e) = result {
                eprintln!("gRPC server error: {}", e);
            }
        });

        Ok((grpc_address, handle))
    }

    pub fn http_endpoint(&self) -> String {
        format!("{}/v1/traces", self.http_address)
    }

    pub fn grpc_endpoint(&self) -> String {
        self.grpc_address.clone()
    }

    pub async fn spans_from_request(
        &self,
        request_idx: usize,
    ) -> Result<SpanCollector, Box<dyn std::error::Error>> {
        let requests = self.requests.lock().await;
        if let Some(request) = requests.get(request_idx) {
            SpanCollector::from_bytes(&request.body)
        } else {
            Err("Request index out of bounds".into())
        }
    }

    pub async fn request_at(&self, request_idx: usize) -> Option<OtlpRequest> {
        let requests = self.requests.lock().await;
        requests.get(request_idx).map(|r| r.clone())
    }

    pub async fn is_empty(&self) -> bool {
        let requests = self.requests.lock().await;
        requests.iter().any(|f| !f.body.is_empty())
    }
}

/// Decode OTLP trace export request from raw protobuf bytes
fn decode_trace_export_request(
    body: &[u8],
) -> Result<ExportTraceServiceRequest, Box<dyn std::error::Error>> {
    Ok(ExportTraceServiceRequest::decode(body)?)
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

pub struct SpanCollector {
    spans: Vec<CollectedSpan>,
    resources: Vec<CollectedResource>,
}

impl SpanCollector {
    /// Create a new span collector from raw OTLP request bytes
    pub fn from_bytes(body: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let request = decode_trace_export_request(body)?;
        let spans = extract_spans(&request);
        let spans = spans.iter().map(|span| span.into()).collect();
        let resources = extract_resource(&request);
        let resources = resources.iter().map(|resource| resource.into()).collect();

        Ok(SpanCollector { spans, resources })
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

    pub fn by_hive_kind(&self, hive_kind: &str) -> Vec<&CollectedSpan> {
        self.spans
            .iter()
            .filter(|s| {
                s.attributes
                    .get("hive.kind")
                    .map_or(false, |v| v == hive_kind)
            })
            .collect()
    }

    pub fn by_hive_kind_one(&self, hive_kind: &str) -> &CollectedSpan {
        let spans: Vec<&CollectedSpan> = self
            .spans
            .iter()
            .filter(|s| {
                s.attributes
                    .get("hive.kind")
                    .map_or(false, |v| v == hive_kind)
            })
            .collect();

        assert_eq!(
            spans.len(),
            1,
            "Expected exactly one span with hive.kind = {}",
            hive_kind
        );

        spans
            .first()
            .expect("Expected exactly one span with hive.kind = {hive_kind}")
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
