use std::{
    num::NonZeroU32,
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
pub struct CancellationToken(tokio_util::sync::CancellationToken, Option<Instant>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(tokio_util::sync::CancellationToken::new(), None)
    }

    pub fn with_timeout(duration: Duration) -> Self {
        let deadline = Instant::now() + duration;
        Self(tokio_util::sync::CancellationToken::new(), Some(deadline))
    }

    pub fn cancel(&self) {
        self.0.cancel();
    }

    #[inline]
    pub fn bail_if_cancelled(&self) -> Result<(), CancellationError> {
        self.bail_if_timedout()?;

        if self.0.is_cancelled() {
            return Err(CancellationError::Cancelled);
        }

        Ok(())
    }

    fn bail_if_timedout(&self) -> Result<(), CancellationError> {
        if let Some(deadline) = self.1 {
            if deadline <= Instant::now() {
                self.cancel();
                return Err(CancellationError::TimedOut);
            }
        }

        Ok(())
    }

    #[inline]
    pub fn throttle_check<'a>(&'a self, every: NonZeroU32) -> CancelTick<'a> {
        CancelTick::new(self, every)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum CancellationError {
    #[error("cancelled")]
    Cancelled,
    #[error("timed out")]
    TimedOut,
}

#[derive(Debug)]
pub struct CancelTick<'a> {
    cancellation_token: &'a CancellationToken,
    every_minus_one: NonZeroU32,
    ticks: u32,
}

impl<'a> CancelTick<'a> {
    #[inline]
    pub fn new(cancellation_token: &'a CancellationToken, every: NonZeroU32) -> Self {
        if !every.is_power_of_two() {
            panic!("every must be a power of two");
        }

        Self {
            cancellation_token,
            every_minus_one: NonZeroU32::new(every.get() - 1).unwrap(),
            ticks: 0,
        }
    }

    #[inline(always)]
    pub fn bail_if_cancelled(&mut self) -> Result<(), CancellationError> {
        // We know that `every_minus_one` is a power of two subtracted by one,
        // that's why we can use bit-and instead of modulo.
        // It's the same as
        //    x % n == 0
        // but cheaper.
        // This is the formula we're using, but the `n-1` is precomputed.
        //    x & (n - 1) == 0
        if self.ticks & self.every_minus_one.get() == 0 {
            self.cancellation_token.bail_if_cancelled()?;
        }
        self.ticks += 1;

        Ok(())
    }
}
