use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::circuit_breaker::CircuitBreakerBuilder;
use crate::primitives::percentage::Percentage;

/// Configuration for the [`recloser`]-backed circuit breaker shared by every
/// component that needs one (subgraph traffic shaping, usage reporting, ...).
///
/// All fields are optional: when `None`, the corresponding default from
/// [`CircuitBreakerBuilder::default`] is used. Components that wrap this
/// type usually add their own outer fields (e.g. an `enabled` toggle, or a
/// list of HTTP status codes that count as failures) on top of it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct CircuitBreakerConfig {
    /// Percentage after what the circuit breaker should kick in.
    /// Default: 50%
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub error_threshold: Option<Percentage>,

    /// Size of the rolling sample used to decide whether the breaker
    /// should open while closed. The breaker fills this sample with the
    /// outcomes of the last `volume_threshold` requests; the next request
    /// after the sample is full is the one whose result is evaluated
    /// against `error_threshold`. In practice the breaker can trip only
    /// after at least `volume_threshold + 1` requests have been observed.
    /// Default: 5
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_threshold: Option<usize>,

    /// The duration after which the circuit breaker will attempt to retry sending requests.
    /// Default: 30s
    #[serde(
        default,
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<String>")]
    pub reset_timeout: Option<Duration>,

    /// Size of the rolling sample of probe requests collected while the
    /// breaker is in the half-open state after `reset_timeout` elapses.
    /// The breaker fills this sample first; the next probe after the
    /// sample is full is the one whose result is evaluated against
    /// `error_threshold` to decide whether to transition back to `closed`
    /// (resuming normal traffic) or to `open` (waiting for another
    /// `reset_timeout` window). In practice at least
    /// `half_open_attempts + 1` probes pass through before the breaker
    /// can transition.
    ///
    /// Lower values make recovery faster but more aggressive; higher
    /// values gather more samples before re-closing the circuit.
    ///
    /// Default: 10
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_open_attempts: Option<usize>,
}

impl CircuitBreakerConfig {
    /// Returns a copy of `self` with each `None` field filled in from
    /// `fallback`. Useful when a subgraph-level config should fall back
    /// to a global config without losing the per-field opt-in semantics.
    pub fn merged_with(&self, fallback: &Self) -> Self {
        Self {
            error_threshold: self.error_threshold.or(fallback.error_threshold),
            volume_threshold: self.volume_threshold.or(fallback.volume_threshold),
            reset_timeout: self.reset_timeout.or(fallback.reset_timeout),
            half_open_attempts: self.half_open_attempts.or(fallback.half_open_attempts),
        }
    }
}

impl From<&CircuitBreakerConfig> for CircuitBreakerBuilder {
    fn from(config: &CircuitBreakerConfig) -> Self {
        let mut builder = CircuitBreakerBuilder::default();
        if let Some(threshold) = config.error_threshold {
            builder = builder.error_threshold(threshold.as_f64() as f32);
        }
        if let Some(volume) = config.volume_threshold {
            builder = builder.volume_threshold(volume);
        }
        if let Some(timeout) = config.reset_timeout {
            builder = builder.reset_timeout(timeout);
        }
        if let Some(attempts) = config.half_open_attempts {
            builder = builder.half_open_attempts(attempts);
        }
        builder
    }
}
