use std::future::Future;
use std::sync::Arc;

use crate::telemetry::logging::request_id::{RequestIdentifiers, REQUEST_IDENTIFIERS};
use crate::telemetry::{
    logging::summary::{RequestSummary, REQUEST_SUMMARY},
    traces::hive_trace_context::{HiveTraceDocuments, HiveTraceFuture, HIVE_TRACE_DOCUMENTS},
};

/// Captures per-request task-locals for work that outlives the request future, such as subscriptions.
pub(crate) struct RequestTaskScope {
    identifiers: Option<Arc<RequestIdentifiers>>,
    summary: Option<Arc<RequestSummary>>,
    hive_trace_documents: Option<Arc<HiveTraceDocuments>>,
}

impl RequestTaskScope {
    pub fn capture() -> Self {
        Self {
            identifiers: REQUEST_IDENTIFIERS.try_with(Arc::clone).ok(),
            summary: REQUEST_SUMMARY.try_with(Arc::clone).ok(),
            hive_trace_documents: HIVE_TRACE_DOCUMENTS.try_with(Arc::clone).ok(),
        }
    }

    pub async fn scope<F: Future>(self, fut: F) -> F::Output {
        let Self {
            identifiers,
            summary,
            hive_trace_documents,
        } = self;
        let fut = async move {
            match (identifiers, summary) {
                (Some(identifiers), Some(summary)) => {
                    REQUEST_IDENTIFIERS
                        .scope(identifiers, REQUEST_SUMMARY.scope(summary, fut))
                        .await
                }
                (Some(identifiers), None) => REQUEST_IDENTIFIERS.scope(identifiers, fut).await,
                (None, Some(summary)) => REQUEST_SUMMARY.scope(summary, fut).await,
                (None, None) => fut.await,
            }
        };
        match hive_trace_documents {
            Some(documents) => HiveTraceFuture::new(fut, documents).await,
            None => fut.await,
        }
    }
}
