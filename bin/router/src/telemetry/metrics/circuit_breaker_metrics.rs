use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use dashmap::DashMap;
use opentelemetry::{
    metrics::{Counter, Meter},
    KeyValue,
};

use crate::telemetry::metrics::catalog::{labels, names, values::CircuitBreakerState};

#[cfg(debug_assertions)]
use crate::telemetry::metrics::catalog::debug_assert_attrs;

/// Tracks circuit-breaker telemetry per subgraph.
///
/// `recloser` does not expose its internal state, so the
/// `state` gauge is derived from observed call outcomes:
/// - any call that the breaker permits (`Ok` or `Err::Inner`) means the
///   underlying state is `Closed` or `HalfOpen` -> reported as `0`,
/// - any call that the breaker rejects (`Err::Rejected`) means the underlying
///   state is `Open` -> reported as `1`.
pub struct CircuitBreakerMetrics {
    short_circuits: Option<Counter<u64>>,
    failures: Option<Counter<u64>>,
    state_transitions: Option<Counter<u64>>,
    /// Per-subgraph state cache storing `0` (closed) or `1` (open). Read by
    /// the `state` observable gauge callback and updated after every breaker
    /// call.
    states: Arc<DashMap<String, Arc<AtomicU8>>>,
}

impl CircuitBreakerMetrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        let states: Arc<DashMap<String, Arc<AtomicU8>>> = Arc::new(DashMap::new());

        let short_circuits = meter.map(|meter| {
            meter
                .u64_counter(names::CIRCUIT_BREAKER_SHORT_CIRCUITS_TOTAL)
                .with_unit("{request}")
                .with_description(
                    "Number of requests rejected by the circuit breaker without \
                     reaching the subgraph (the breaker was open).",
                )
                .build()
        });

        let failures = meter.map(|meter| {
            meter
                .u64_counter(names::CIRCUIT_BREAKER_FAILURES_TOTAL)
                .with_unit("{request}")
                .with_description(
                    "Number of subgraph requests counted as failures by the \
                     circuit breaker (errors or configured failure status codes).",
                )
                .build()
        });

        let state_transitions = meter.map(|meter| {
            meter
                .u64_counter(names::CIRCUIT_BREAKER_STATE_TRANSITIONS_TOTAL)
                .with_unit("{transition}")
                .with_description("Number of circuit breaker state transitions per subgraph.")
                .build()
        });

        if let Some(meter) = meter {
            let states_for_callback = states.clone();
            meter
                .u64_observable_gauge(names::CIRCUIT_BREAKER_STATE)
                .with_description(
                    "Current circuit breaker state per subgraph (0 = closed, 1 = open).",
                )
                .with_callback(move |observer| {
                    for entry in states_for_callback.iter() {
                        let attributes =
                            [KeyValue::new(labels::SUBGRAPH_NAME, entry.key().clone())];

                        #[cfg(debug_assertions)]
                        debug_assert_attrs(names::CIRCUIT_BREAKER_STATE, &attributes);

                        observer.observe(entry.value().load(Ordering::Relaxed) as u64, &attributes);
                    }
                })
                .build();
        }

        Self {
            short_circuits,
            failures,
            state_transitions,
            states,
        }
    }

    /// Eagerly registers a subgraph so that the `state` gauge reports a `0`
    /// (closed) baseline even when no traffic has been observed yet. Called
    /// once per subgraph when a circuit breaker is configured for it.
    pub fn register_subgraph(&self, subgraph_name: &str) {
        self.state_entry(subgraph_name);
    }

    /// Records that the breaker rejected a request (short-circuit). Also
    /// updates the cached state to `open` and emits a transition event if the
    /// state was previously `closed`.
    pub fn record_short_circuit(&self, subgraph_name: &str) {
        let attributes = [KeyValue::new(
            labels::SUBGRAPH_NAME,
            subgraph_name.to_string(),
        )];

        if let Some(counter) = &self.short_circuits {
            #[cfg(debug_assertions)]
            debug_assert_attrs(names::CIRCUIT_BREAKER_SHORT_CIRCUITS_TOTAL, &attributes);
            counter.add(1, &attributes);
        }

        self.set_state(subgraph_name, CircuitBreakerState::Open);
    }

    /// Records that the breaker permitted a call that ultimately failed
    /// (`Err::Inner`). The call was attempted, so the breaker is `closed`
    /// or `half_open` from our perspective.
    pub fn record_failure(&self, subgraph_name: &str) {
        let attributes = [KeyValue::new(
            labels::SUBGRAPH_NAME,
            subgraph_name.to_string(),
        )];

        if let Some(counter) = &self.failures {
            #[cfg(debug_assertions)]
            debug_assert_attrs(names::CIRCUIT_BREAKER_FAILURES_TOTAL, &attributes);
            counter.add(1, &attributes);
        }

        self.set_state(subgraph_name, CircuitBreakerState::Closed);
    }

    /// Records that the breaker permitted a successful call. Marks the state
    /// as `closed`.
    pub fn record_success(&self, subgraph_name: &str) {
        self.set_state(subgraph_name, CircuitBreakerState::Closed);
    }

    /// Atomically updates the cached state for a subgraph. When the value
    /// actually changes, emits a `state_transitions_total` data point.
    fn set_state(&self, subgraph_name: &str, new_state: CircuitBreakerState) {
        let entry = self.state_entry(subgraph_name);
        let new_value = new_state.as_u8();
        let previous = entry.swap(new_value, Ordering::AcqRel);
        if previous != new_value {
            self.record_state_transition(
                subgraph_name,
                CircuitBreakerState::from_u8(previous),
                new_state,
            );
        }
    }

    fn record_state_transition(
        &self,
        subgraph_name: &str,
        from: CircuitBreakerState,
        to: CircuitBreakerState,
    ) {
        let Some(counter) = &self.state_transitions else {
            return;
        };

        let attributes = [
            KeyValue::new(labels::SUBGRAPH_NAME, subgraph_name.to_string()),
            KeyValue::new(labels::CIRCUIT_BREAKER_FROM_STATE, from.as_str()),
            KeyValue::new(labels::CIRCUIT_BREAKER_TO_STATE, to.as_str()),
        ];

        #[cfg(debug_assertions)]
        debug_assert_attrs(names::CIRCUIT_BREAKER_STATE_TRANSITIONS_TOTAL, &attributes);

        counter.add(1, &attributes);
    }

    fn state_entry(&self, subgraph_name: &str) -> Arc<AtomicU8> {
        if let Some(existing) = self.states.get(subgraph_name) {
            return existing.clone();
        }
        self.states
            .entry(subgraph_name.to_string())
            .or_insert_with(|| Arc::new(AtomicU8::new(CircuitBreakerState::Closed.as_u8())))
            .clone()
    }
}
