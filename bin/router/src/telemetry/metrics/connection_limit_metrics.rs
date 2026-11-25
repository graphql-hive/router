//! Metrics for `traffic_shaping.max_connections_per_host`.

use std::time::Instant;

use opentelemetry::{
    metrics::{Histogram, Meter, UpDownCounter},
    KeyValue,
};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;
use crate::telemetry::metrics::catalog::{labels, names, values::SlotWaitResult};

pub struct ConnectionLimitMetrics {
    instruments: Option<SlotWaitInstruments>,
}

#[derive(Clone)]
struct SlotWaitInstruments {
    pending_requests: UpDownCounter<i64>,
    wait_duration: Histogram<f64>,
}

impl ConnectionLimitMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let instruments = meter.map(|meter| SlotWaitInstruments {
            pending_requests: meter
                .i64_up_down_counter(names::SUBGRAPH_REQUESTS_PENDING)
                .with_unit("{request}")
                .with_description(
                    "Number of subgraph requests waiting for a free slot because \
                     `traffic_shaping.max_connections_per_host` was reached.",
                )
                .build(),
            wait_duration: meter
                .f64_histogram(names::SUBGRAPH_REQUEST_SLOT_WAIT_DURATION)
                .with_unit("s")
                .with_description(
                    "How long subgraph requests waited for a free slot because \
                     `traffic_shaping.max_connections_per_host` was reached, by how the \
                     wait ended.",
                )
                .build(),
        });

        Self { instruments }
    }

    /// Returns the slot-wait recorder for one subgraph endpoint, so its attributes are built
    /// once per executor instead of on every wait.
    pub fn slot_wait(
        &self,
        subgraph_name: &str,
        server_address: &str,
        server_port: u16,
    ) -> SlotWaitMetrics {
        let Some(instruments) = &self.instruments else {
            return SlotWaitMetrics::disabled();
        };

        let endpoint_attributes = [
            KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
            KeyValue::new(labels::SERVER_ADDRESS, server_address.to_string()),
            KeyValue::new(labels::SERVER_PORT, i64::from(server_port)),
        ];
        let with_result = |result: SlotWaitResult| {
            let [subgraph_name, server_address, server_port] = endpoint_attributes.clone();
            [
                subgraph_name,
                server_address,
                server_port,
                KeyValue::new(labels::RESULT, result.as_str()),
            ]
        };
        let result_attributes = ResultAttributes {
            acquired: with_result(SlotWaitResult::Acquired),
            timeout: with_result(SlotWaitResult::Timeout),
            cancelled: with_result(SlotWaitResult::Cancelled),
        };

        #[cfg(debug_assertions)]
        {
            debug_assert_attrs(names::SUBGRAPH_REQUESTS_PENDING, &endpoint_attributes);
            for result in SlotWaitResult::ALL {
                debug_assert_attrs(
                    names::SUBGRAPH_REQUEST_SLOT_WAIT_DURATION,
                    result_attributes.get(result),
                );
            }
        }

        SlotWaitMetrics {
            recorder: Some(SlotWaitRecorder {
                instruments: instruments.clone(),
                endpoint_attributes,
                result_attributes,
            }),
        }
    }
}

/// Records the waits of one subgraph endpoint's requests for a slot.
pub struct SlotWaitMetrics {
    recorder: Option<SlotWaitRecorder>,
}

struct SlotWaitRecorder {
    instruments: SlotWaitInstruments,
    endpoint_attributes: [KeyValue; 3],
    result_attributes: ResultAttributes,
}

/// The duration attributes for each [`SlotWaitResult`], built once per endpoint.
struct ResultAttributes {
    acquired: [KeyValue; 4],
    timeout: [KeyValue; 4],
    cancelled: [KeyValue; 4],
}

impl ResultAttributes {
    fn get(&self, result: SlotWaitResult) -> &[KeyValue; 4] {
        match result {
            SlotWaitResult::Acquired => &self.acquired,
            SlotWaitResult::Timeout => &self.timeout,
            SlotWaitResult::Cancelled => &self.cancelled,
        }
    }
}

impl SlotWaitMetrics {
    pub fn disabled() -> Self {
        Self { recorder: None }
    }

    /// Counts one request as pending until the returned guard is dropped, so the count goes
    /// back down however the wait ends, and then records how long it waited. The wait is
    /// recorded as cancelled unless [`SlotWait::acquired`] or [`SlotWait::timed_out`] is called.
    pub fn start(&self) -> SlotWait<'_> {
        let started = self.recorder.as_ref().map(|recorder| {
            recorder
                .instruments
                .pending_requests
                .add(1, &recorder.endpoint_attributes);
            Instant::now()
        });
        SlotWait {
            metrics: self,
            started,
            result: SlotWaitResult::Cancelled,
        }
    }
}

pub struct SlotWait<'a> {
    metrics: &'a SlotWaitMetrics,
    /// `None` when metrics are disabled, so the clock is never read.
    started: Option<Instant>,
    result: SlotWaitResult,
}

impl SlotWait<'_> {
    pub fn acquired(&mut self) {
        self.result = SlotWaitResult::Acquired;
    }

    pub fn timed_out(&mut self) {
        self.result = SlotWaitResult::Timeout;
    }
}

impl Drop for SlotWait<'_> {
    fn drop(&mut self) {
        let (Some(recorder), Some(started)) = (&self.metrics.recorder, self.started) else {
            return;
        };
        recorder
            .instruments
            .pending_requests
            .add(-1, &recorder.endpoint_attributes);
        recorder.instruments.wait_duration.record(
            started.elapsed().as_secs_f64(),
            recorder.result_attributes.get(self.result),
        );
    }
}
