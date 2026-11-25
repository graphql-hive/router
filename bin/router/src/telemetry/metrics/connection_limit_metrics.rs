//! Metrics for `traffic_shaping.max_connections_per_host`.

use opentelemetry::{
    metrics::{Meter, UpDownCounter},
    KeyValue,
};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names};

pub struct ConnectionLimitMetrics {
    pending_requests: Option<UpDownCounter<i64>>,
}

impl ConnectionLimitMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let pending_requests = meter.map(|meter| {
            meter
                .i64_up_down_counter(names::SUBGRAPH_REQUESTS_PENDING)
                .with_unit("{request}")
                .with_description(
                    "Number of subgraph requests waiting for a free slot because \
                     `traffic_shaping.max_connections_per_host` was reached.",
                )
                .build()
        });

        Self { pending_requests }
    }

    /// Returns the pending-requests recorder for one subgraph endpoint, so its attributes are
    /// built once per executor instead of on every wait.
    pub fn pending_requests(&self, subgraph_name: &str, endpoint: &http::Uri) -> PendingRequests {
        let Some(counter) = &self.pending_requests else {
            return PendingRequests::disabled();
        };

        let server_port = endpoint
            .port_u16()
            .unwrap_or_else(|| match endpoint.scheme_str() {
                Some("https") => 443,
                _ => 80,
            });
        let attributes = [
            KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
            KeyValue::new(
                labels::SERVER_ADDRESS,
                endpoint.host().unwrap_or("unknown").to_string(),
            ),
            KeyValue::new(labels::SERVER_PORT, i64::from(server_port)),
        ];

        #[cfg(debug_assertions)]
        debug_assert_attrs(names::SUBGRAPH_REQUESTS_PENDING, &attributes);

        PendingRequests {
            recorder: Some((counter.clone(), attributes)),
        }
    }
}

/// Counts the requests of one subgraph endpoint that wait for a slot.
pub struct PendingRequests {
    recorder: Option<(UpDownCounter<i64>, [KeyValue; 3])>,
}

impl PendingRequests {
    pub fn disabled() -> Self {
        Self { recorder: None }
    }

    /// Counts one request as pending until the returned guard is dropped, so the count goes
    /// back down however the wait ends: with a slot, a timeout or a cancelled request.
    pub fn start(&self) -> PendingRequestGuard<'_> {
        if let Some((counter, attributes)) = &self.recorder {
            counter.add(1, attributes);
        }
        PendingRequestGuard { pending: self }
    }
}

pub struct PendingRequestGuard<'a> {
    pending: &'a PendingRequests,
}

impl Drop for PendingRequestGuard<'_> {
    fn drop(&mut self) {
        if let Some((counter, attributes)) = &self.pending.recorder {
            counter.add(-1, attributes);
        }
    }
}
