use opentelemetry::{metrics::Counter, metrics::Meter, KeyValue};

use crate::telemetry::metrics::catalog::{labels, names};

pub struct CircuitBreakerMetrics {
    rejected_requests: Option<Counter<u64>>,
}

impl CircuitBreakerMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let rejected_requests = meter.map(|meter| {
            meter
                .u64_counter(names::CIRCUIT_BREAKER_REJECTED_REQUESTS)
                .with_unit("{request}")
                .with_description("Number of requests rejected by circuit breaker")
                .build()
        });

        Self { rejected_requests }
    }

    /// Records a rejected request due to circuit breaker being open
    pub fn record_rejected_request(&self, subgraph_name: &str) {
        if let Some(counter) = &self.rejected_requests {
            counter.add(
                1,
                &[KeyValue::new(
                    labels::SUBGRAPH_NAME,
                    subgraph_name.to_string(),
                )],
            );
        }
    }
}
