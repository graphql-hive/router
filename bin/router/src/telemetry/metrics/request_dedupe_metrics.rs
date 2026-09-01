use opentelemetry::metrics::{Counter, Meter};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::names;

#[derive(Clone)]
pub struct RequestDedupeMetrics {
    joined_total: Option<Counter<u64>>,
}

impl RequestDedupeMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let joined_total = meter.map(|meter| {
            meter
                .u64_counter(names::REQUEST_DEDUPE_JOINED_TOTAL)
                .with_unit("{request}")
                .with_description(
                    "Number of inbound requests that were deduplicated by joining an \
                     already in-flight request instead of executing their own.",
                )
                .build()
        });

        Self { joined_total }
    }

    /// Records that an inbound request joined an already in-flight deduplicated request.
    pub fn record_joined(&self) {
        let Some(counter) = &self.joined_total else {
            return;
        };

        #[cfg(debug_assertions)]
        debug_assert_attrs(names::REQUEST_DEDUPE_JOINED_TOTAL, &[]);
        counter.add(1, &[]);
    }
}
