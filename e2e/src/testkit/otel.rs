pub use opentelemetry_proto::tonic::trace::v1::span::SpanKind;
use std::fmt::Display;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::trace::v1::{Span, Status};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct TraceParent<'a> {
    pub trace_id: &'a str,
    pub span_id: &'a str,
}

impl<'a> TraceParent<'a> {
    /// Generates W3C Trace Context traceparent header value.
    /// Format: "00-{trace_id}-{span_id}-{trace_flags}"
    pub fn to_string(&self) -> String {
        format!("00-{}-{}-01", self.trace_id, self.span_id)
    }

    pub fn parse(traceparent: &'a str) -> Self {
        let parts: Vec<&str> = traceparent.split('-').collect();
        assert_eq!(parts.len(), 4, "traceparent should have 4 parts");

        Self {
            trace_id: parts[1],
            span_id: parts[2],
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

/// A simplified span representation for snapshot testing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotSpan {
    pub id: String,
    pub name: String,
    pub kind: Option<SpanKind>,
    pub status: Option<Status>,
    pub duration_ms: f64,
    pub attributes: BTreeMap<String, String>,
    pub events: Vec<SnapshotEvent>,
}

/// A simplified event representation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotEvent {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
}

/// Mock OTLP collector server that captures incoming trace export requests
pub struct OtlpCollector {
    pub address: String,
    requests: Arc<Mutex<Vec<OtlpRequest>>>,
    _handle: std::thread::JoinHandle<()>,
}

impl OtlpCollector {
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let server = tiny_http::Server::http("127.0.0.1:0")
            .map_err(|e| format!("Failed to start OTLP server: {}", e))?;
        let address_str = format!("http://{}", server.server_addr());

        info!("OTLP collector server starting on {}", address_str);

        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_clone = requests.clone();

        // Spawn server in a background thread (tiny_http is blocking)
        let handle = std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let method = request.method().to_string();
                let path = request.url().to_string();

                // Extract headers
                let headers: Vec<(String, String)> = request
                    .headers()
                    .iter()
                    .map(|h| (h.field.to_string(), h.value.to_string()))
                    .collect();

                // Read body
                let mut body = Vec::new();
                let _ = request.as_reader().read_to_end(&mut body);

                info!("Captured OTLP request: {} {}", method, path);

                // Store request (need to block here since we're in a thread)
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    requests_clone.lock().await.push(OtlpRequest {
                        method,
                        path,
                        headers,
                        body,
                    });
                });
            }
        });

        Ok(OtlpCollector {
            address: address_str,
            requests,
            _handle: handle,
        })
    }

    pub fn traces_endpoint(&self) -> String {
        format!("{}/v1/traces", self.address)
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
}

impl Drop for OtlpCollector {
    fn drop(&mut self) {
        // Server thread will terminate when server is dropped
    }
}

/// Decode OTLP trace export request from raw protobuf bytes
fn decode_trace_export_request(
    body: &[u8],
) -> Result<ExportTraceServiceRequest, Box<dyn std::error::Error>> {
    let request = ExportTraceServiceRequest::decode(body)?;
    Ok(request)
}

/// Extract all spans from a trace export request
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

/// Convert a span to a snapshot-friendly representation
fn span_to_snapshot(span: &Span) -> SnapshotSpan {
    let duration_nanos = span.end_time_unix_nano - span.start_time_unix_nano;
    let duration_ms = duration_nanos as f64 / 1_000_000.0;

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
        .map(|event| SnapshotEvent {
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

    SnapshotSpan {
        id: hex::encode(&span.span_id),
        name: span.name.clone(),
        kind: format_span_kind(span.kind),
        status: span.status.clone(),
        duration_ms,
        attributes,
        events,
    }
}

fn format_span_kind(kind: i32) -> Option<SpanKind> {
    SpanKind::try_from(kind).ok()
}

fn format_attribute_value(value: &opentelemetry_proto::tonic::common::v1::AnyValue) -> String {
    use opentelemetry_proto::tonic::common::v1::any_value::Value;

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

/// Helper struct for span filtering and assertion
pub struct SpanCollector {
    spans: Vec<SnapshotSpan>,
}

impl SpanCollector {
    /// Create a new span collector from raw OTLP request bytes
    pub fn from_bytes(body: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let request = decode_trace_export_request(body)?;
        let spans = extract_spans(&request);
        let snapshot_spans = spans.iter().map(span_to_snapshot).collect();

        Ok(SpanCollector {
            spans: snapshot_spans,
        })
    }

    /// Find spans by name prefix
    pub fn by_hive_kind(&self, hive_kind: &str) -> Vec<&SnapshotSpan> {
        self.spans
            .iter()
            .filter(|s| {
                s.attributes
                    .get("hive.kind")
                    .map_or(false, |v| v == hive_kind)
            })
            .collect()
    }

    pub fn by_id(&self, id: &str) -> Option<&SnapshotSpan> {
        self.spans.iter().find(|s| s.id == id)
    }
}

impl Display for SnapshotSpan {
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
                writeln!(f, "  Status: Node")?;
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
