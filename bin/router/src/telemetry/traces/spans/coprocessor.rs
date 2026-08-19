use tracing::{info_span, Level, Span};

use crate::telemetry::traces::{disabled_span, is_level_enabled, spans::TARGET_NAME};

pub struct CoprocessorSpan {
    pub span: Span,
}

impl std::ops::Deref for CoprocessorSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl CoprocessorSpan {
    pub fn new(stage: &'static str, id: &str) -> Self {
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let span = info_span!(
            target: TARGET_NAME,
            "coprocessor",
            "hive.kind" = "coprocessor",
            "otel.kind" = "Internal",
            "coprocessor.stage" = stage,
            "coprocessor.id" = id,
        );
        CoprocessorSpan { span }
    }
}
