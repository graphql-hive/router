use std::time::Duration;

use opentelemetry::{
    metrics::{Counter, Histogram, Meter},
    KeyValue,
};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names, values};

pub struct CacheWarmupMetrics {
    operations_total: Option<Counter<u64>>,
    duration: Option<Histogram<f64>>,
}

impl CacheWarmupMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        Self {
            operations_total: meter.map(|meter| {
                meter
                    .u64_counter(names::PLAN_CACHE_WARMUP_OPERATIONS_TOTAL)
                    .with_description(
                        "Operations considered by plan cache warmup, by where they came from and what happened to them",
                    )
                    .build()
            }),
            duration: meter.map(|meter| {
                meter
                    .f64_histogram(names::PLAN_CACHE_WARMUP_DURATION)
                    .with_unit("s")
                    .with_description("How long plan cache warmup held back the switch to a new supergraph")
                    .build()
            }),
        }
    }

    pub fn record_operations(
        &self,
        source: values::CacheWarmupSource,
        result: values::CacheWarmupOperationResult,
        count: u64,
    ) {
        let Some(counter) = &self.operations_total else {
            return;
        };
        if count == 0 {
            return;
        }
        let attributes = [
            KeyValue::new(labels::SOURCE, source.as_str()),
            KeyValue::new(labels::RESULT, result.as_str()),
        ];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::PLAN_CACHE_WARMUP_OPERATIONS_TOTAL, &attributes);
        counter.add(count, &attributes);
    }

    pub fn record_run(&self, result: values::CacheWarmupResult, elapsed: Duration) {
        let Some(histogram) = &self.duration else {
            return;
        };
        let attributes = [KeyValue::new(labels::RESULT, result.as_str())];
        #[cfg(debug_assertions)]
        debug_assert_attrs(names::PLAN_CACHE_WARMUP_DURATION, &attributes);
        histogram.record(elapsed.as_secs_f64(), &attributes);
    }
}
