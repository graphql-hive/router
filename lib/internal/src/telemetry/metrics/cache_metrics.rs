use std::time::Instant;

use opentelemetry::{
    metrics::{Counter, Histogram, Meter},
    KeyValue,
};

use crate::telemetry::metrics::capture::Capture;
#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names, values};

#[derive(Clone)]
pub struct CacheMetrics {
    pub parse: CacheMetricSet,
    pub validate: CacheMetricSet,
    pub normalize: CacheMetricSet,
    pub plan: CacheMetricSet,
}

impl CacheMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        Self {
            parse: CacheMetricSet::new(
                meter,
                names::PARSE_CACHE_REQUESTS_TOTAL,
                names::PARSE_CACHE_DURATION,
                names::PARSE_CACHE_SIZE,
                "Parse",
            ),
            validate: CacheMetricSet::new(
                meter,
                names::VALIDATE_CACHE_REQUESTS_TOTAL,
                names::VALIDATE_CACHE_DURATION,
                names::VALIDATE_CACHE_SIZE,
                "Validate",
            ),
            normalize: CacheMetricSet::new(
                meter,
                names::NORMALIZE_CACHE_REQUESTS_TOTAL,
                names::NORMALIZE_CACHE_DURATION,
                names::NORMALIZE_CACHE_SIZE,
                "Normalize",
            ),
            plan: CacheMetricSet::new(
                meter,
                names::PLAN_CACHE_REQUESTS_TOTAL,
                names::PLAN_CACHE_DURATION,
                names::PLAN_CACHE_SIZE,
                "Plan",
            ),
        }
    }
}

#[derive(Clone)]
struct CacheInstruments {
    requests_total: Option<Counter<u64>>,
    duration: Option<Histogram<f64>>,
    #[cfg(debug_assertions)]
    requests_metric_name: &'static str,
    #[cfg(debug_assertions)]
    duration_metric_name: &'static str,
    size_metric_name: &'static str,
    size_metric_description: String,
    meter: Option<Meter>,
}

impl CacheInstruments {
    fn is_enabled(&self) -> bool {
        self.requests_total.is_some() || self.duration.is_some()
    }
}

#[derive(Clone)]
pub struct CacheMetricSet {
    instruments: CacheInstruments,
}

pub struct CacheRequestState<'a> {
    instruments: &'a CacheInstruments,
    started_at: Instant,
}

impl CacheMetricSet {
    fn new(
        meter: Option<&Meter>,
        requests_metric_name: &'static str,
        duration_metric_name: &'static str,
        size_metric_name: &'static str,
        metric_description_prefix: &'static str,
    ) -> Self {
        let requests_total = meter.map(|meter| {
            meter
                .u64_counter(requests_metric_name)
                .with_description(format!("{} requests", metric_description_prefix))
                .build()
        });
        let duration = meter.map(|meter| {
            meter
                .f64_histogram(duration_metric_name)
                .with_unit("s")
                .with_description(format!("{} duration", metric_description_prefix))
                .build()
        });
        Self {
            instruments: CacheInstruments {
                requests_total,
                duration,
                #[cfg(debug_assertions)]
                requests_metric_name,
                #[cfg(debug_assertions)]
                duration_metric_name,
                meter: meter.cloned(),
                size_metric_name,
                size_metric_description: format!("{} size", metric_description_prefix),
            },
        }
    }

    pub fn hit(&self, duration: std::time::Duration) {
        self.record_request(values::CacheResult::Hit, duration);
    }

    pub fn miss(&self, duration: std::time::Duration) {
        self.record_request(values::CacheResult::Miss, duration);
    }

    pub fn capture_request<'a>(&'a self) -> Capture<CacheRequestState<'a>> {
        if !self.instruments.is_enabled() {
            return Capture::disabled();
        }

        Capture::enabled(CacheRequestState {
            instruments: &self.instruments,
            started_at: Instant::now(),
        })
    }

    pub fn observe_size_with(&self, size_fn: impl Fn() -> u64 + Send + Sync + 'static) {
        if !self.instruments.is_enabled() {
            return;
        }
        let Some(meter) = &self.instruments.meter else {
            return;
        };
        #[cfg(debug_assertions)]
        debug_assert_attrs(self.instruments.size_metric_name, &[]);
        meter
            .u64_observable_gauge(self.instruments.size_metric_name)
            .with_description(self.instruments.size_metric_description.clone())
            .with_callback(move |observer| {
                observer.observe(size_fn(), &[]);
            })
            .build();
    }

    fn record_request(&self, result: values::CacheResult, duration: std::time::Duration) {
        if !self.instruments.is_enabled() {
            return;
        }

        let attributes = [KeyValue::new(labels::RESULT, result.as_str())];

        if let Some(counter) = &self.instruments.requests_total {
            #[cfg(debug_assertions)]
            debug_assert_attrs(self.instruments.requests_metric_name, &attributes);
            counter.add(1, &attributes);
        }
        if let Some(histogram) = &self.instruments.duration {
            #[cfg(debug_assertions)]
            debug_assert_attrs(self.instruments.duration_metric_name, &attributes);
            histogram.record(duration.as_secs_f64(), &attributes);
        }
    }
}

impl Capture<CacheRequestState<'_>> {
    pub fn finish_hit(self) {
        self.record(values::CacheResult::Hit);
    }

    pub fn finish_miss(self) {
        self.record(values::CacheResult::Miss);
    }

    fn record(self, result: values::CacheResult) {
        let Some(state) = self.take() else {
            return;
        };

        let attributes = [KeyValue::new(labels::RESULT, result.as_str())];

        if let Some(counter) = &state.instruments.requests_total {
            #[cfg(debug_assertions)]
            debug_assert_attrs(state.instruments.requests_metric_name, &attributes);
            counter.add(1, &attributes);
        }

        if let Some(histogram) = &state.instruments.duration {
            #[cfg(debug_assertions)]
            debug_assert_attrs(state.instruments.duration_metric_name, &attributes);
            histogram.record(state.started_at.elapsed().as_secs_f64(), &attributes);
        }
    }
}
