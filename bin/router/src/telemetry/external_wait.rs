//! Per-request accounting of the time the router spends waiting on external
//! services (subgraphs, coprocessors) - the "idle" part of a request.
//!
//! Waits can overlap (parallel plan nodes fan out to several subgraphs at
//! once), so the tracker measures the *union* of the wait intervals rather
//! than their sum.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::task::futures::TaskLocalFuture;

tokio::task_local! {
    static EXTERNAL_WAIT: Arc<ExternalWaitTracker>;
}

#[derive(Default)]
pub struct ExternalWaitTracker {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Number of external waits currently in flight.
    depth: u32,
    /// When the current open span started (`depth > 0`).
    open_since: Option<Instant>,
    /// Sum of all closed spans so far.
    total: Duration,
}

impl ExternalWaitTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn enter(self: &Arc<Self>) -> ExternalWaitGuard {
        self.enter_at(Instant::now())
    }

    fn enter_at(self: &Arc<Self>, now: Instant) -> ExternalWaitGuard {
        let mut inner = self.lock();
        if inner.depth == 0 {
            inner.open_since = Some(now);
        }
        inner.depth += 1;
        ExternalWaitGuard {
            tracker: Arc::clone(self),
        }
    }

    fn exit_at(&self, now: Instant) {
        let mut inner = self.lock();
        debug_assert!(
            inner.depth > 0,
            "external wait exited more times than entered"
        );
        inner.depth = inner.depth.saturating_sub(1);
        if inner.depth == 0 {
            if let Some(open_since) = inner.open_since.take() {
                inner.total += now.saturating_duration_since(open_since);
            }
        }
    }

    pub fn total_at(&self, now: Instant) -> Duration {
        let inner = self.lock();
        match inner.open_since {
            Some(open_since) => inner.total + now.saturating_duration_since(open_since),
            None => inner.total,
        }
    }

    pub fn total(&self) -> Duration {
        self.total_at(Instant::now())
    }
}

/// Closes the external wait it was created for when dropped.
pub struct ExternalWaitGuard {
    tracker: Arc<ExternalWaitTracker>,
}

impl Drop for ExternalWaitGuard {
    fn drop(&mut self) {
        self.tracker.exit_at(Instant::now());
    }
}

pub fn enter() -> Option<ExternalWaitGuard> {
    EXTERNAL_WAIT.try_with(ExternalWaitTracker::enter).ok()
}

pub trait WithExternalWaitTracker: Future + Sized {
    fn with_external_wait_tracker(
        self,
        tracker: Arc<ExternalWaitTracker>,
    ) -> TaskLocalFuture<Arc<ExternalWaitTracker>, Self> {
        EXTERNAL_WAIT.scope(tracker, self)
    }
}

impl<F: Future> WithExternalWaitTracker for F {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn empty_tracker_has_zero_total() {
        let tracker = ExternalWaitTracker::new();
        assert_eq!(tracker.total(), Duration::ZERO);
    }

    #[test]
    fn sequential_waits_are_summed() {
        let tracker = ExternalWaitTracker::new();
        let t0 = Instant::now();

        let guard = tracker.enter_at(t0);
        tracker.exit_at(t0 + ms(100));
        std::mem::forget(guard);

        let guard = tracker.enter_at(t0 + ms(200));
        tracker.exit_at(t0 + ms(250));
        std::mem::forget(guard);

        assert_eq!(tracker.total_at(t0 + ms(300)), ms(150));
    }

    #[test]
    fn overlapping_waits_are_unioned_not_summed() {
        let tracker = ExternalWaitTracker::new();
        let t0 = Instant::now();

        // a: [0, 100], b: [50, 300] -> union [0, 300] = 300, sum would be 350
        let a = tracker.enter_at(t0);
        let b = tracker.enter_at(t0 + ms(50));
        tracker.exit_at(t0 + ms(100));
        std::mem::forget(a);
        tracker.exit_at(t0 + ms(300));
        std::mem::forget(b);

        assert_eq!(tracker.total_at(t0 + ms(400)), ms(300));
    }

    #[test]
    fn fully_nested_wait_does_not_add_time() {
        let tracker = ExternalWaitTracker::new();
        let t0 = Instant::now();

        // outer: [0, 500], inner: [100, 200] -> 500
        let outer = tracker.enter_at(t0);
        let inner = tracker.enter_at(t0 + ms(100));
        tracker.exit_at(t0 + ms(200));
        std::mem::forget(inner);
        tracker.exit_at(t0 + ms(500));
        std::mem::forget(outer);

        assert_eq!(tracker.total_at(t0 + ms(600)), ms(500));
    }

    #[test]
    fn open_span_is_counted_up_to_now() {
        let tracker = ExternalWaitTracker::new();
        let t0 = Instant::now();

        let guard = tracker.enter_at(t0);
        assert_eq!(tracker.total_at(t0 + ms(70)), ms(70));
        std::mem::forget(guard);
    }

    #[test]
    fn guard_drop_closes_the_wait() {
        let tracker = ExternalWaitTracker::new();
        {
            let _guard = tracker.enter();
            assert_eq!(tracker.lock().depth, 1);
        }
        assert_eq!(tracker.lock().depth, 0);
        assert!(tracker.lock().open_since.is_none());
    }

    #[tokio::test]
    async fn enter_is_a_noop_outside_a_request_scope() {
        assert!(enter().is_none());
    }

    #[tokio::test]
    async fn enter_records_into_the_scoped_tracker() {
        let tracker = ExternalWaitTracker::new();
        async {
            let guard = enter().expect("tracker should be in scope");
            assert_eq!(EXTERNAL_WAIT.with(|t| t.lock().depth), 1);
            drop(guard);
            assert_eq!(EXTERNAL_WAIT.with(|t| t.lock().depth), 0);
        }
        .with_external_wait_tracker(tracker.clone())
        .await;
    }

    #[tokio::test]
    async fn cancelled_future_releases_its_wait() {
        let tracker = ExternalWaitTracker::new();
        async {
            let pending = async {
                let _guard = enter();
                std::future::pending::<()>().await;
            };
            // Dropping the pending future drops the guard it holds.
            let _ = tokio::time::timeout(Duration::from_millis(5), pending).await;
            assert_eq!(EXTERNAL_WAIT.with(|t| t.lock().depth), 0);
        }
        .with_external_wait_tracker(tracker.clone())
        .await;
        assert!(tracker.total() >= Duration::from_millis(5));
    }
}
