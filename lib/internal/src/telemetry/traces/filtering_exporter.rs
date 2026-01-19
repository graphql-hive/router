use opentelemetry::Value;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use std::collections::HashSet;

use super::spans::attributes::OTEL_DROP;

/// A span exporter that filters out entire traces when any span is marked with `otel.drop=true`
#[derive(Debug)]
pub struct FilteringSpanExporter<E: SpanExporter> {
    inner: E,
}

impl<E: SpanExporter> FilteringSpanExporter<E> {
    pub fn new(inner: E) -> Self {
        Self { inner }
    }
}

impl<E: SpanExporter> SpanExporter for FilteringSpanExporter<E> {
    async fn export(&self, mut batch: Vec<SpanData>) -> OTelSdkResult {
        let mut trace_ids_to_drop = HashSet::new();

        for span in &batch {
            for kv in span.attributes.iter() {
                if kv.key.as_str() == OTEL_DROP && matches!(&kv.value, Value::Bool(true)) {
                    trace_ids_to_drop.insert(span.span_context.trace_id());
                    break;
                }
            }
        }

        if trace_ids_to_drop.is_empty() {
            self.inner.export(batch).await?;
            return Ok(());
        }

        // Filter out all spans belonging to dropped traces
        batch.retain(|span| !trace_ids_to_drop.contains(&span.span_context.trace_id()));

        if !batch.is_empty() {
            self.inner.export(batch).await?;
        }

        Ok(())
    }

    fn shutdown(&mut self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn force_flush(&mut self) -> OTelSdkResult {
        self.inner.force_flush()
    }
}
