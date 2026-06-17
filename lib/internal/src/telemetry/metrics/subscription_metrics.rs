use opentelemetry::{
    metrics::{Counter, Meter},
    KeyValue,
};

use crate::telemetry::metrics::catalog::{labels, names};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;

/// Telemetry for GraphQL subscriptions.
pub struct SubscriptionMetrics {
    dropped_events_total: Option<Counter<u64>>,
}

impl SubscriptionMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let dropped_events_total = meter.map(|meter| {
            meter
                .u64_counter(names::SUBSCRIPTION_DROPPED_EVENTS_TOTAL)
                .with_unit("{event}")
                .with_description(
                    "Number of subscription events dropped because the consumer could not keep \
                     up with the upstream subgraph (drop-oldest back-pressure).",
                )
                .build()
        });

        Self {
            dropped_events_total,
        }
    }

    /// Records that an upstream subscription event was dropped for a subgraph because the
    /// consumer fell behind and a newer event replaced it.
    pub fn record_dropped_event(&self, subgraph_name: &str) {
        let Some(counter) = &self.dropped_events_total else {
            return;
        };

        let attributes = [KeyValue::new(
            labels::SUBGRAPH_NAME,
            subgraph_name.to_string(),
        )];

        #[cfg(debug_assertions)]
        debug_assert_attrs(names::SUBSCRIPTION_DROPPED_EVENTS_TOTAL, &attributes);

        counter.add(1, &attributes);
    }
}
