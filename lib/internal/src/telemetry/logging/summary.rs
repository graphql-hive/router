use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::future::Future;
use std::rc::Rc;
use std::sync::atomic::{
    AtomicBool, AtomicI64, AtomicU16, AtomicU32, AtomicU64, Ordering::Relaxed,
};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use ntex::http::body::{Body, BodySize, MessageBody, ResponseBody};
use ntex::util::Bytes;
use ntex::web::HttpResponse;
use tokio::task::futures::TaskLocalFuture;
use tracing::{info, Level};

use crate::telemetry::logging::request_id::{RequestIdentifiers, REQUEST_IDENTIFIERS};
use crate::telemetry::logging::targets;

#[derive(Default)]
pub struct RequestSummary {
    pub client_name: OnceLock<String>,
    pub client_version: OnceLock<String>,
    pub operation_name: OnceLock<String>,
    pub operation_type: OnceLock<&'static str>,
    pub operation_hash: OnceLock<String>,
    pub persisted_document_id: OnceLock<String>,
    pub subgraph_requests: AtomicU32,
    pub involved_subgraphs: Mutex<HashSet<String>>,
    pub error_count: AtomicU32,
    pub partial_response: AtomicBool,
    pub response_code: OnceLock<&'static str>,
    pub response_mode: OnceLock<&'static str>,
    pub status_code: AtomicU16,
    pub payload_bytes: AtomicI64,
    pub duration_ms: AtomicU64,
    pub supergraph_identifier: AtomicU64,
    pub custom: Mutex<BTreeMap<String, sonic_rs::Value>>,
    pub message: OnceLock<Cow<'static, str>>,
}

impl RequestSummary {
    pub fn set_client_info(&self, client_name: Option<&str>, client_version: Option<&str>) {
        set_once(&self.client_name, client_name);
        set_once(&self.client_version, client_version);
    }

    pub fn set_error_count(&self, error_count: u32) {
        self.error_count
            .store(error_count, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_operation_name(&self, operation_name: Option<&str>) {
        set_once(&self.operation_name, operation_name);
    }

    pub fn set_operation_type(&self, operation_type: &'static str) {
        let _ = self.operation_type.set(operation_type);
    }

    pub fn set_operation_hash(&self, operation_hash: Option<&str>) {
        set_once(&self.operation_hash, operation_hash);
    }

    pub fn set_persisted_document_id(&self, persisted_document_id: Option<&str>) {
        set_once(&self.persisted_document_id, persisted_document_id);
    }

    pub fn set_partial_response(&self, partial: bool) {
        self.partial_response.store(partial, Relaxed);
    }

    pub fn set_response_code(&self, code: &'static str) {
        let _ = self.response_code.set(code);
    }

    pub fn set_response_mode(&self, mode: &'static str) {
        let _ = self.response_mode.set(mode);
    }

    pub fn set_duration(&self, duration: Duration) {
        self.duration_ms.store(duration.as_millis() as u64, Relaxed);
    }

    pub fn set_supergraph_identifier(&self, identifier: u64) {
        self.supergraph_identifier.store(identifier, Relaxed);
    }

    /// Records a plugin-contributed attribute, so it's included when the summary is emitted.
    /// Setting the same key again overwrites the previous value.
    pub fn set_custom(&self, key: impl Into<String>, value: impl Into<sonic_rs::Value>) {
        if let Ok(mut custom) = self.custom.lock() {
            custom.insert(key.into(), value.into());
        }
    }

    /// Overrides the summary log line's message. First call wins; later calls are no-ops.
    pub fn set_message(&self, message: impl Into<Cow<'static, str>>) {
        let _ = self.message.set(message.into());
    }

    pub fn record_subgraph(&self, name: &str) {
        self.subgraph_requests.fetch_add(1, Relaxed);
        if let Ok(mut subgraphs) = self.involved_subgraphs.lock() {
            if !subgraphs.contains(name) {
                subgraphs.insert(name.to_string());
            }
        }
    }

    pub fn emit(&self) {
        let involved_subgraphs = self
            .involved_subgraphs
            .lock()
            .map(|subgraphs| {
                let mut names: Vec<&str> = subgraphs.iter().map(String::as_str).collect();
                names.sort_unstable();
                names.join(",")
            })
            .unwrap_or_default();

        info!(
            target: targets::SUMMARY,
            message = self.message.get().map(Cow::as_ref),
            client_name = self.client_name.get().map(String::as_str),
            client_version = self.client_version.get().map(String::as_str),
            operation_name = self.operation_name.get().map(String::as_str),
            operation_type = self.operation_type.get().copied(),
            operation_hash = self.operation_hash.get().map(String::as_str),
            persisted_document_id = self.persisted_document_id.get().map(String::as_str),
            subgraph_requests = self.subgraph_requests.load(Relaxed),
            involved_subgraphs = involved_subgraphs.as_str(),
            error_count = self.error_count.load(Relaxed),
            partial_response = self.partial_response.load(Relaxed),
            error_code = self.response_code.get().copied(),
            response_mode = self.response_mode.get().copied(),
            status_code = self.status_code.load(Relaxed),
            payload_bytes = self.payload_bytes.load(Relaxed),
            supergraph_identifier = self.supergraph_identifier.load(Relaxed),
            duration_ms = self.duration_ms.load(Relaxed),
        );
    }
}

fn set_once(cell: &OnceLock<String>, value: Option<&str>) {
    if let Some(value) = value {
        let _ = cell.set(value.to_string());
    }
}

tokio::task_local! {
    pub static REQUEST_SUMMARY: Arc<RequestSummary>;
}

/// Whether the summary log target is live. A single cached callsite, so when the
/// target is filtered off (e.g. `router::request=off`) this is a cheap atomic load.
#[inline]
pub fn is_enabled() -> bool {
    tracing::enabled!(target: targets::SUMMARY, Level::INFO)
}

pub fn record(f: impl FnOnce(&RequestSummary)) {
    if !is_enabled() {
        return;
    }
    let _ = REQUEST_SUMMARY.try_with(|summary| f(summary));
}

pub fn current_summary() -> Option<Arc<RequestSummary>> {
    REQUEST_SUMMARY.try_with(|summary| summary.clone()).ok()
}

pub fn emit() {
    if !is_enabled() {
        return;
    }
    let _ = REQUEST_SUMMARY.try_with(|summary| summary.emit());
}

fn disabled_summary() -> Arc<RequestSummary> {
    static DISABLED: OnceLock<Arc<RequestSummary>> = OnceLock::new();
    DISABLED
        .get_or_init(|| Arc::new(RequestSummary::default()))
        .clone()
}

pub trait WithRequestSummary: Future + Sized {
    fn with_request_summary(self) -> TaskLocalFuture<Arc<RequestSummary>, Self> {
        let summary = if is_enabled() {
            Arc::new(RequestSummary::default())
        } else {
            disabled_summary()
        };
        REQUEST_SUMMARY.scope(summary, self)
    }
}

impl<F: Future> WithRequestSummary for F {}

/// Emits the request summary when dropped, recording request duration from `started_at`.
///
/// The summary and the request identifiers are captured by value, so the guard can outlive the
/// task-local scope of the request - required for streamed responses, whose body is polled by
/// the server long after the handler future resolved.
pub struct SummaryOnDrop {
    started_at: std::time::Instant,
    summary: Option<Arc<RequestSummary>>,
    pub request_ids: Option<Arc<RequestIdentifiers>>,
}

impl SummaryOnDrop {
    pub fn new(started_at: std::time::Instant) -> Self {
        let (summary, request_ids) = if is_enabled() {
            (
                REQUEST_SUMMARY.try_with(Arc::clone).ok(),
                REQUEST_IDENTIFIERS.try_with(Arc::clone).ok(),
            )
        } else {
            (None, None)
        };

        Self {
            started_at,
            summary,
            request_ids,
        }
    }

    /// Moves the guard into a streamed response body, so the summary is emitted when the stream
    /// terminates (or the client disconnects) instead of when the response was built.
    pub fn attach_to_response(self, response: HttpResponse) -> HttpResponse {
        response.map_body(|_, body| {
            ResponseBody::Body(Body::from_message(SummaryTrackedBody {
                body,
                summary: self,
                payload_bytes: 0,
            }))
        })
    }

    fn record(&self, f: impl FnOnce(&RequestSummary)) {
        if let Some(summary) = &self.summary {
            f(summary);
        }
    }
}

impl Drop for SummaryOnDrop {
    fn drop(&mut self) {
        let Some(summary) = self.summary.take() else {
            return;
        };
        summary.set_duration(self.started_at.elapsed());

        // Re-enter both task-locals before emitting: by now (especially for responses whose
        // body outlives the original request future) they may no longer be ambiently scoped,
        // but the formatters look up `custom`/`correlations` independently via their own
        // `try_with` at format time, so both must be active for that lookup to succeed.
        let request_ids = self.request_ids.take();
        REQUEST_SUMMARY.sync_scope(summary, || match request_ids {
            Some(ids) => REQUEST_IDENTIFIERS.sync_scope(ids, emit),
            None => emit(),
        });
    }
}

/// Used to track the body of a response, so the summary is emitted when the stream
/// terminates (or the client disconnects) instead of when the response was built.
/// Streamed bodies have no known size upfront, so the bytes sent to the client are also summed here.
struct SummaryTrackedBody {
    body: ResponseBody<Body>,
    summary: SummaryOnDrop,
    payload_bytes: i64,
}

impl MessageBody for SummaryTrackedBody {
    fn size(&self) -> BodySize {
        self.body.size()
    }

    fn poll_next_chunk(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Rc<dyn Error>>>> {
        let poll = self.body.poll_next_chunk(cx);
        if let Poll::Ready(Some(Ok(chunk))) = &poll {
            self.payload_bytes = self.payload_bytes.saturating_add(chunk.len() as i64);
        }
        poll
    }
}

impl Drop for SummaryTrackedBody {
    fn drop(&mut self) {
        // runs before the `SummaryOnDrop` field is dropped, so the total is part of the summary
        let payload_bytes = self.payload_bytes;
        self.summary
            .record(|s| s.payload_bytes.store(payload_bytes, Relaxed));
    }
}
