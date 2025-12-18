use fastrace::prelude::Span;
use std::sync::atomic::{AtomicBool, Ordering};

static TRACING_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_tracing_enabled(enabled: bool) {
    TRACING_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_tracing_enabled() -> bool {
    TRACING_ENABLED.load(Ordering::Relaxed)
}

pub fn disabled_span() -> Span {
    Span::noop()
}
