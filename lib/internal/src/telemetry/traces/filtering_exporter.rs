use moka::sync::Cache;
use opentelemetry::trace::TraceId;
use opentelemetry::Value;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use std::collections::HashSet;
use std::time::Duration;

use super::spans::attributes::OTEL_DROP;

/// A span exporter that filters out entire traces when any span is marked with `otel.drop=true`.
/// It uses an LRU cache to remember dropped trace IDs across batches.
#[derive(Debug)]
pub struct FilteringSpanExporter<E: SpanExporter> {
    inner: E,
    dropped_traces_cache: Cache<TraceId, ()>,
}

impl<E: SpanExporter> FilteringSpanExporter<E> {
    pub fn new(inner: E) -> Self {
        Self {
            inner,
            dropped_traces_cache: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(60))
                .build(),
        }
    }
}

impl<E: SpanExporter> SpanExporter for FilteringSpanExporter<E> {
    async fn export(&self, mut batch: Vec<SpanData>) -> OTelSdkResult {
        let mut trace_ids_to_drop = HashSet::new();

        for span in &batch {
            for kv in span.attributes.iter() {
                if kv.key.as_str() == OTEL_DROP && matches!(&kv.value, Value::Bool(true)) {
                    let trace_id = span.span_context.trace_id();
                    trace_ids_to_drop.insert(trace_id);
                    self.dropped_traces_cache.insert(trace_id, ());
                    break;
                }
            }
        }

        if trace_ids_to_drop.is_empty() && self.dropped_traces_cache.weighted_size() == 0 {
            self.inner.export(batch).await?;
            return Ok(());
        }

        // Filter out all spans belonging to dropped traces
        batch.retain(|span| {
            let trace_id = span.span_context.trace_id();
            !trace_ids_to_drop.contains(&trace_id)
                && !self.dropped_traces_cache.contains_key(&trace_id)
        });

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

    fn set_resource(&mut self, res: &opentelemetry_sdk::Resource) {
        self.inner.set_resource(res);
    }
}
