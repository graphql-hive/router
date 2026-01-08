use std::sync::atomic::{AtomicU8, Ordering};
use tracing::{Level, Span};

// Atomic representation of the max enabled tracing level.
// 0: Off, 1: Error, 2: Warn, 3: Info, 4: Debug, 5: Trace
static MAX_LEVEL: AtomicU8 = AtomicU8::new(3); // Default to Info

#[derive(Debug, Clone, Copy)]
pub enum TelemetryLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

pub fn set_tracing_level(level: TelemetryLevel) {
    MAX_LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn set_tracing_enabled(enabled: bool) {
    if enabled {
        set_tracing_level(TelemetryLevel::Info);
    } else {
        set_tracing_level(TelemetryLevel::Off);
    }
}

pub fn is_level_enabled(level: Level) -> bool {
    let current = MAX_LEVEL.load(Ordering::Relaxed);
    let requested = match level {
        Level::ERROR => 1,
        Level::WARN => 2,
        Level::INFO => 3,
        Level::DEBUG => 4,
        Level::TRACE => 5,
    };
    current >= requested
}

pub fn is_tracing_enabled() -> bool {
    MAX_LEVEL.load(Ordering::Relaxed) > 0
}

pub fn disabled_span() -> Span {
    Span::none()
}
