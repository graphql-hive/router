use std::time::Instant;

use opentelemetry::{
    metrics::{Counter, Histogram, Meter},
    KeyValue,
};

use crate::telemetry::metrics::capture::Capture;
#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names, values};

struct SupergraphInstruments {
    poll_total: Option<Counter<u64>>,
    poll_duration: Option<Histogram<f64>>,
    process_duration: Option<Histogram<f64>>,
}

impl SupergraphInstruments {
    fn is_poll_enabled(&self) -> bool {
        self.poll_total.is_some() || self.poll_duration.is_some()
    }

    fn is_process_enabled(&self) -> bool {
        self.process_duration.is_some()
    }
}

pub struct SupergraphPollState<'a> {
    instruments: &'a SupergraphInstruments,
    started_at: Instant,
}

pub struct SupergraphProcessState<'a> {
    instruments: &'a SupergraphInstruments,
    started_at: Instant,
}

pub struct SupergraphMetrics {
    instruments: SupergraphInstruments,
}

impl SupergraphMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let poll_total = meter.map(|meter| {
            meter
                .u64_counter(names::SUPERGRAPH_POLL_TOTAL)
                .with_description("Total number of supergraph poll attempts")
                .build()
        });
        let poll_duration = meter.map(|meter| {
            meter
                .f64_histogram(names::SUPERGRAPH_POLL_DURATION)
                .with_unit("s")
                .with_description("Duration of supergraph poll processing")
                .build()
        });

        let process_duration = meter.map(|meter| {
            meter
                .f64_histogram(names::SUPERGRAPH_PROCESS_DURATION)
                .with_unit("s")
                .with_description("Duration of supergraph processing")
                .build()
        });

        Self {
            instruments: SupergraphInstruments {
                poll_total,
                poll_duration,
                process_duration,
            },
        }
    }

    pub fn capture_poll<'a>(&'a self) -> Capture<SupergraphPollState<'a>> {
        if !self.instruments.is_poll_enabled() {
            return Capture::disabled();
        }

        Capture::enabled(SupergraphPollState {
            instruments: &self.instruments,
            started_at: Instant::now(),
        })
    }

    pub fn capture_process<'a>(&'a self) -> Capture<SupergraphProcessState<'a>> {
        if !self.instruments.is_process_enabled() {
            return Capture::disabled();
        }

        Capture::enabled(SupergraphProcessState {
            instruments: &self.instruments,
            started_at: Instant::now(),
        })
    }
}

impl<'a> Capture<SupergraphPollState<'a>> {
    pub fn finish_not_modified(self) {
        self.record(values::SupergraphPollResult::NotModified);
    }

    pub fn finish_updated(self) {
        self.record(values::SupergraphPollResult::Updated);
    }

    pub fn finish_error(self) {
        self.record(values::SupergraphPollResult::Error);
    }

    fn record(self, result: values::SupergraphPollResult) {
        let Some(state) = self.take() else {
            return;
        };

        let attributes = [KeyValue::new(labels::RESULT, result.as_str())];

        if let Some(counter) = &state.instruments.poll_total {
            #[cfg(debug_assertions)]
            debug_assert_attrs(names::SUPERGRAPH_POLL_TOTAL, &attributes);
            counter.add(1, &attributes);
        }

        if let Some(histogram) = &state.instruments.poll_duration {
            #[cfg(debug_assertions)]
            debug_assert_attrs(names::SUPERGRAPH_POLL_DURATION, &attributes);
            histogram.record(state.started_at.elapsed().as_secs_f64(), &attributes);
        }
    }
}

impl<'a> Capture<SupergraphProcessState<'a>> {
    pub fn finish_ok(self) {
        self.record(values::SupergraphProcessStatus::Ok);
    }

    pub fn finish_error(self) {
        self.record(values::SupergraphProcessStatus::Error);
    }

    fn record(self, status: values::SupergraphProcessStatus) {
        let Some(state) = self.take() else {
            return;
        };

        if let Some(histogram) = &state.instruments.process_duration {
            let attributes = [KeyValue::new(labels::STATUS, status.as_str())];
            #[cfg(debug_assertions)]
            debug_assert_attrs(names::SUPERGRAPH_PROCESS_DURATION, &attributes);
            histogram.record(state.started_at.elapsed().as_secs_f64(), &attributes);
        }
    }
}
