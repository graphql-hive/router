use opentelemetry::{
    metrics::{Counter, Meter},
    KeyValue,
};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names, values};

struct GraphQLInstruments {
    errors_total: Option<Counter<u64>>,
}

pub struct GraphQLMetrics {
    instruments: GraphQLInstruments,
}

#[derive(Clone)]
pub struct GraphQLErrorMetricsRecorder {
    counter: Counter<u64>,
}

impl GraphQLMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let errors_total = meter.map(|meter| {
            meter
                .u64_counter(names::GRAPHQL_ERRORS_TOTAL)
                .with_unit("{error}")
                .with_description("Total number of GraphQL errors in responses")
                .build()
        });

        Self {
            instruments: GraphQLInstruments { errors_total },
        }
    }

    pub fn error_recorder(&self) -> Option<GraphQLErrorMetricsRecorder> {
        self.instruments
            .errors_total
            .as_ref()
            .cloned()
            .map(|counter| GraphQLErrorMetricsRecorder { counter })
    }

    pub fn record_error(&self, code: &str) {
        if let Some(recorder) = self.error_recorder() {
            recorder.record_error_code(Some(code));
        }
    }
}

impl GraphQLErrorMetricsRecorder {
    pub fn record_error_code(&self, code: Option<&str>) {
        let code = code
            .filter(|code| !code.is_empty())
            .unwrap_or(values::UNKNOWN);
        let attributes = [KeyValue::new(labels::CODE, code.to_string())];

        #[cfg(debug_assertions)]
        debug_assert_attrs(names::GRAPHQL_ERRORS_TOTAL, &attributes);
        self.counter.add(1, &attributes);
    }

    pub fn record_errors<'a, Fn, It>(&self, errors_fn: Fn)
    where
        Fn: FnOnce() -> It,
        It: IntoIterator<Item = Option<&'a str>>,
    {
        for code in errors_fn() {
            self.record_error_code(code);
        }
    }
}
