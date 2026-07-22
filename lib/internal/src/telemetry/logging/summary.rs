use std::collections::HashSet;
use std::future::Future;
use std::sync::atomic::{
    AtomicBool, AtomicI64, AtomicU16, AtomicU32, AtomicU64, Ordering::Relaxed,
};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::task::futures::TaskLocalFuture;
use tracing::{info, Level};

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
