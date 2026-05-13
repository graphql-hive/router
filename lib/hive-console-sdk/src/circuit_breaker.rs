use std::time::Duration;

use recloser::{AsyncRecloser, Recloser};

#[derive(Clone)]
pub struct CircuitBreakerBuilder {
    error_threshold: f32,
    volume_threshold: usize,
    reset_timeout: Duration,
    half_open_attempts: usize,
}

impl Default for CircuitBreakerBuilder {
    fn default() -> Self {
        Self {
            error_threshold: 0.5,
            volume_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            half_open_attempts: 10,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CircuitBreakerError {
    #[error("Invalid error threshold: {0}. It must be between 0.0 and 1.0")]
    InvalidErrorThreshold(f32),
}

impl CircuitBreakerBuilder {
    /// Percentage after what the circuit breaker should kick in.
    /// Default: .5
    pub fn error_threshold(mut self, percentage: f32) -> Self {
        self.error_threshold = percentage;
        self
    }
    /// Size of the rolling sample used to decide whether the breaker
    /// should open while closed. The breaker fills this sample with the
    /// outcomes of the last `volume_threshold` requests; the next request
    /// after the sample is full is the one whose result is evaluated
    /// against `error_threshold`. In practice this means at least
    /// `volume_threshold + 1` requests must be observed before the
    /// breaker can trip.
    /// Default: 5
    pub fn volume_threshold(mut self, threshold: usize) -> Self {
        self.volume_threshold = threshold;
        self
    }
    /// After what time the circuit breaker is attempting to retry sending requests in milliseconds.
    /// Default: 30s
    pub fn reset_timeout(mut self, timeout: Duration) -> Self {
        self.reset_timeout = timeout;
        self
    }
    /// Size of the rolling sample of probe requests collected while the
    /// breaker is in the `HalfOpen` state after `reset_timeout` elapses.
    /// The breaker fills this sample first; the next probe after the
    /// sample is full is the one whose result is evaluated against
    /// `error_threshold` to decide whether to transition back to `Closed`
    /// or `Open`. In practice this means at least `half_open_attempts + 1`
    /// probes pass through before the breaker can transition.
    /// Default: 10
    pub fn half_open_attempts(mut self, attempts: usize) -> Self {
        self.half_open_attempts = attempts;
        self
    }

    pub fn build_async(self) -> Result<AsyncRecloser, CircuitBreakerError> {
        let recloser = self.build_sync()?;
        Ok(AsyncRecloser::from(recloser))
    }
    pub fn build_sync(self) -> Result<Recloser, CircuitBreakerError> {
        let error_threshold = if self.error_threshold < 0.0 || self.error_threshold > 1.0 {
            return Err(CircuitBreakerError::InvalidErrorThreshold(
                self.error_threshold,
            ));
        } else {
            self.error_threshold
        };
        let recloser = Recloser::custom()
            .error_rate(error_threshold)
            .closed_len(self.volume_threshold)
            .half_open_len(self.half_open_attempts)
            .open_wait(self.reset_timeout)
            .build();
        Ok(recloser)
    }
}
