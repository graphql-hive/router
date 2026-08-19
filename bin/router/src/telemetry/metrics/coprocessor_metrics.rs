use opentelemetry::metrics::{Counter, Histogram, Meter};
use opentelemetry::KeyValue;

use crate::telemetry::metrics::catalog;
#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;

pub struct CoprocessorMetrics {
    pub requests_total: Option<Counter<u64>>,
    pub duration: Option<Histogram<f64>>,
    pub errors_total: Option<Counter<u64>>,
}

impl CoprocessorMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let Some(meter) = meter else {
            return Self {
                requests_total: None,
                duration: None,
                errors_total: None,
            };
        };

        Self {
            requests_total: Some(
                meter
                    .u64_counter(catalog::names::COPROCESSOR_REQUESTS_TOTAL)
                    .with_description("Total number of coprocessor requests")
                    .build(),
            ),
            duration: Some(
                meter
                    .f64_histogram(catalog::names::COPROCESSOR_DURATION)
                    .with_description("Duration of coprocessor requests")
                    .with_unit("s")
                    .build(),
            ),
            errors_total: Some(
                meter
                    .u64_counter(catalog::names::COPROCESSOR_ERRORS_TOTAL)
                    .with_description("Total number of coprocessor errors")
                    .build(),
            ),
        }
    }

    pub fn record_request(&self, stage: &'static str) {
        if let Some(metric) = &self.requests_total {
            let attrs = [KeyValue::new(catalog::labels::COPROCESSOR_STAGE, stage)];
            #[cfg(debug_assertions)]
            debug_assert_attrs(catalog::names::COPROCESSOR_REQUESTS_TOTAL, &attrs);
            metric.add(1, &attrs);
        }
    }

    pub fn record_duration(&self, stage: &'static str, duration: f64) {
        if let Some(metric) = &self.duration {
            let attrs = [KeyValue::new(catalog::labels::COPROCESSOR_STAGE, stage)];
            #[cfg(debug_assertions)]
            debug_assert_attrs(catalog::names::COPROCESSOR_DURATION, &attrs);
            metric.record(duration, &attrs);
        }
    }

    pub fn record_error(&self, stage: &'static str) {
        if let Some(metric) = &self.errors_total {
            let attrs = [KeyValue::new(catalog::labels::COPROCESSOR_STAGE, stage)];
            #[cfg(debug_assertions)]
            debug_assert_attrs(catalog::names::COPROCESSOR_ERRORS_TOTAL, &attrs);
            metric.add(1, &attrs);
        }
    }
}
