use std::future::Future;
use std::sync::Arc;

use crate::telemetry::logging::request_id::{RequestIdentifiers, REQUEST_IDENTIFIERS};
use crate::telemetry::logging::summary::{RequestSummary, REQUEST_SUMMARY};

/// Snapshot of the per-request logging task-locals, so work that outlives the request future (=subscriptions) can
/// keep logging and recording under the request it originated from.
pub struct RequestLogScope {
    identifiers: Option<Arc<RequestIdentifiers>>,
    summary: Option<Arc<RequestSummary>>,
}

impl RequestLogScope {
    pub fn capture() -> Self {
        Self {
            identifiers: REQUEST_IDENTIFIERS.try_with(Arc::clone).ok(),
            summary: REQUEST_SUMMARY.try_with(Arc::clone).ok(),
        }
    }

    pub async fn scope<F: Future>(self, fut: F) -> F::Output {
        match (self.identifiers, self.summary) {
            (Some(identifiers), Some(summary)) => {
                REQUEST_IDENTIFIERS
                    .scope(identifiers, REQUEST_SUMMARY.scope(summary, fut))
                    .await
            }
            (Some(identifiers), None) => REQUEST_IDENTIFIERS.scope(identifiers, fut).await,
            (None, Some(summary)) => REQUEST_SUMMARY.scope(summary, fut).await,
            (None, None) => fut.await,
        }
    }
}
