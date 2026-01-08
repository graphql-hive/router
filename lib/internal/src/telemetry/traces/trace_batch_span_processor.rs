//! Trace-batching span processor that buffers spans per trace and exports them
//! when the trace is considered "complete" (the root span ended).
//!
//! # Overview
//!
//! Unlike standard batch processors that export spans as soon as a buffer fills up,
//! this processor groups spans by their `TraceId`.
//! A trace is considered ready for export when the root span has ended
//! (indicating the request is finished).
//!
//! TraceBatchSpanProcessor - it sends spans via a channel to a background worker.
//! TraceAggregator - The background worker running on a Tokio task. It manages the buffer,
//!                   grouping, lifetime cleanup, and spawns parallel export tasks.
//!
//! This separation ensures that heavy work does not block the other threads emitting spans.
//!
//! ## Why batch by trace?
//! The `HiveConsoleExporter` rewrites relationships (promotes `graphql.operation` to root,
//! moves HTTP attributes/timing from `http.server`, and maps `http.client`/`http.inflight`
//! to `graphql.subgraph.operation`) and drops traces that are missing those parents or
//! children. That transformation needs the complete set of spans for a trace available at
//! once. Emitting spans early (standard batch processing) would fragment traces and cause the
//! exporter to mis-normalize or discard partial data.

use ahash::AHashMap;
use hive_router_config::telemetry::hive::TraceBatchProcessorConfig;
use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry::Context;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{Span, SpanData, SpanExporter, SpanProcessor};
use opentelemetry_sdk::Resource;
use tokio::select;
use tokio::task::JoinSet;
use tokio::{
    sync::{mpsc, RwLock},
    time,
};
use tracing::{debug, error, warn};

use std::fmt;
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::telemetry::error::TelemetryError;

/// Messages sent between Router threads and batch span processor's work thread
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum BatchMessage {
    /// Export spans, usually called when span ends
    ExportSpan(SpanData),
    /// Flush the current buffer to the backend, it can be triggered by
    /// pre configured interval or a call to `force_push` function.
    Flush(Option<std::sync::mpsc::Sender<OTelSdkResult>>),
    /// Shut down the worker thread, push all spans in buffer to the backend.
    Shutdown(std::sync::mpsc::Sender<OTelSdkResult>),
    /// Set the resource for the exporter.
    SetResource(Arc<Resource>),
}

/// Per-trace state tracking buffered spans and completion status.
#[derive(Debug)]
struct ActiveTrace {
    spans: Vec<SpanData>,
    /// Timestamp when the root span ended (if any).
    root_end_time: Option<Instant>,
    /// When the first span of this trace ended.
    first_seen: Instant,
}

impl ActiveTrace {
    fn new(now: Instant, capacity: usize) -> Self {
        Self {
            spans: Vec::with_capacity(capacity),
            root_end_time: None,
            first_seen: now,
        }
    }

    /// Returns true once the root span has ended.
    #[inline]
    fn is_complete(&self) -> bool {
        self.root_end_time.is_some()
    }

    /// Checks if the trace has been kept in memory for longer than `max_lifetime`
    /// regardless of completion status.
    fn lifetime_exceeded(&self, now: Instant, max_lifetime: Duration) -> bool {
        now.duration_since(self.first_seen) > max_lifetime
    }

    fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    fn add_span(
        &mut self,
        span: SpanData,
        now: Instant,
        config: &Config,
        counters: &DropMetrics,
    ) -> bool {
        let is_root = span.parent_span_id == SpanId::INVALID || span.parent_span_is_remote;
        let root_ended_now = is_root && !self.is_complete();

        if root_ended_now {
            self.root_end_time = Some(now);
        }

        if self.spans.len() < config.max_spans_per_trace {
            self.spans.push(span);
            return root_ended_now;
        }

        let max_spans_per_trace = config.max_spans_per_trace;
        warn!(
            name: "TraceBatchingProcessor.SpanDiscarded",
            trace_id = %span.span_context.trace_id(),
            %max_spans_per_trace,
            "Span discarded due to maximum spans per trace limit"
        );

        counters
            .dropped_spans_per_trace_limit_count
            .fetch_add(1, Ordering::Relaxed);

        root_ended_now
    }
}

/// Shared counters for tracking dropped items across threads.
#[derive(Debug, Default)]
struct DropMetrics {
    dropped_spans_count: AtomicUsize,
    dropped_spans_per_trace_limit_count: AtomicUsize,
}

#[derive(Debug, Clone)]
struct Config {
    // TODO: when we implement some kind of Client -> Router timeout, let's change this value
    /// Maximum duration to hold a trace in memory from the moment its first span arrives.
    ///
    /// This acts as a hard timeout to ensure that traces without a root end
    /// (missing or lost) are eventually cleaned up.
    pub max_trace_lifetime: Duration,
    /// Interval for the background sweeper task.
    pub sweep_interval: Duration,
    pub max_traces_in_memory: usize,
    pub max_spans_per_trace: usize,
    pub max_export_timeout: Duration,
    pub max_export_batch_size: usize,
    pub scheduled_delay: Duration,
    pub max_concurrent_exports: usize,
}

/// Trace-batching span processor that exports traces from an export queue.
pub struct TraceBatchSpanProcessor {
    message_sender: mpsc::Sender<BatchMessage>,
    drop_metrics: Arc<DropMetrics>,
    config: Config,
}

impl fmt::Debug for TraceBatchSpanProcessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraceBatchSpanProcessor")
            .field("message_sender", &self.message_sender)
            .finish()
    }
}

impl SpanProcessor for TraceBatchSpanProcessor {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {
        // no-op
    }

    fn on_end(&self, span: SpanData) {
        if !span.span_context.is_sampled() {
            return;
        }

        let result = self.message_sender.try_send(BatchMessage::ExportSpan(span));

        if result.is_err() {
            let count = self
                .drop_metrics
                .dropped_spans_count
                .fetch_add(1, Ordering::Relaxed);

            if count == 0 {
                warn!(
                  name: "TraceBatchSpanProcessor.SpanDroppingStarted",
                  message = "Beginning to drop span messages due to full/internal errors. No further log will be emitted for next 100 spans. During Shutdown time, a log will be emitted with exact count of total spans dropped."
                );
            } else if count.is_multiple_of(100) {
                warn!(
                  name: "TraceBatchSpanProcessor.SpanDroppingStarted",
                  message = "Still droping span messages due to full/internal errors. No further log will be emitted for next 100 spans. During Shutdown time, a log will be emitted with exact count of total spans dropped."
                );
            }
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        let (res_sender, res_receiver) = std::sync::mpsc::channel();
        self.message_sender
            .try_send(BatchMessage::Flush(Some(res_sender)))
            .map_err(|err| {
                OTelSdkError::InternalFailure(format!("Failed to send flush message: {err}"))
            })?;

        res_receiver.recv().map_err(|err| {
            OTelSdkError::InternalFailure(format!("Flush response channel error: {err}"))
        })?
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        let total_dropped = self
            .drop_metrics
            .dropped_spans_count
            .load(Ordering::Relaxed);
        let limit_dropped = self
            .drop_metrics
            .dropped_spans_per_trace_limit_count
            .load(Ordering::Relaxed);

        if total_dropped > 0 || limit_dropped > 0 {
            let max_traces_in_memory = self.config.max_traces_in_memory;
            warn!(
                name: "BatchSpanProcessor.Shutdown",
                total_spans_dropped = total_dropped,
                spans_dropped_trace_limit = limit_dropped,
                max_traces_in_memory = max_traces_in_memory,
                message = "Shutdown complete. Dropped spans statistics."
            );
        }

        let (res_sender, res_receiver) = std::sync::mpsc::channel();
        self.message_sender
            .try_send(BatchMessage::Shutdown(res_sender))
            .map_err(|err| {
                OTelSdkError::InternalFailure(format!("Failed to send shutdown message: {err}"))
            })?;

        res_receiver.recv().map_err(|err| {
            OTelSdkError::InternalFailure(format!("Shutdown response channel error: {err}"))
        })?
    }

    fn set_resource(&mut self, resource: &Resource) {
        let resource = Arc::new(resource.clone());
        let _ = self
            .message_sender
            .try_send(BatchMessage::SetResource(resource));
    }
}

/// Aggregates spans by trace, decides when traces are ready,
/// and exports them directly.
struct TraceAggregator<E> {
    /// The actual exporter (Filter -> Hive -> OTLP exporter) protected by a lock for thread safety.
    exporter: Arc<RwLock<E>>,
    /// A set of background tasks currently exporting data.
    /// This allows exports to happen concurrently without blocking the main loop.
    export_tasks: JoinSet<OTelSdkResult>,
    config: Config,
    /// Map of TraceId -> ActiveTrace.
    /// Stores spans for traces that are currently being collected.
    active_traces: AHashMap<TraceId, ActiveTrace>,
    /// Traces that have finished (root span ended) but haven't been exported yet.
    /// They are buffered here until we have enough for a batch or a timer fires.
    export_queue: Vec<ActiveTrace>,
    /// Cached current time to avoid calling `Instant::now()` too frequently.
    cached_time: Instant,
    drop_metrics: Arc<DropMetrics>,
}

impl<E: SpanExporter + 'static> TraceAggregator<E> {
    fn enqueue_for_export(&mut self, trace: ActiveTrace) {
        if trace.is_empty() {
            return;
        }

        self.export_queue.push(trace);
    }

    fn enqueue_completed(&mut self, trace_id: TraceId) {
        if let Some(trace) = self.active_traces.remove(&trace_id) {
            self.enqueue_for_export(trace);
        }
    }

    /// Process a single message received from the channel.
    /// Returns `false` if the processor should shut down.
    async fn process_message(&mut self, message: BatchMessage) -> bool {
        match message {
            // Adds span to active traces.
            BatchMessage::ExportSpan(span) => {
                self.handle_span(span);
                // Trigger export if we have enough traces to fill a batch
                if self.export_queue.len() >= self.config.max_export_batch_size {
                    self.flush_export_queue().await;
                }
            }
            // Forces all queued traces to be exported immediately.
            BatchMessage::Flush(channel) => {
                self.sweep();
                self.flush_export_queue().await;
                // Wait for our exports
                self.wait_for_exports().await;
                // Trigger downstream flush
                let _ = self.exporter.write().await.force_flush();

                if let Some(channel) = channel {
                    let _ = channel.send(Ok(()));
                }
            }
            // Drains everything and shuts down the exporter.
            BatchMessage::Shutdown(channel) => {
                self.drain_active_traces();
                self.flush_export_queue().await;
                tracing::info!(
                    component = "telemetry",
                    layer = "trace_batch_processor",
                    "shutdown scheduled"
                );
                let result = self.exporter.write().await.shutdown();
                tracing::info!(
                    component = "telemetry",
                    layer = "trace_batch_processor",
                    "shutdown completed"
                );
                let _ = channel.send(result);
                return false;
            }
            // Updates the exporter resource.
            BatchMessage::SetResource(resource) => {
                self.exporter.write().await.set_resource(&resource);
            }
        }
        true
    }

    async fn export(
        batch: Vec<SpanData>,
        exporter: Arc<RwLock<E>>,
        max_export_timeout: Duration,
    ) -> OTelSdkResult {
        if batch.is_empty() {
            return Ok(());
        }

        // Acquires Read Lock.
        // Multiple tasks can hold this lock simultaneously, allowing concurrent exports.
        let exporter_guard = exporter.read().await;
        let export_fut = exporter_guard.export(batch);

        select! {
             res = export_fut => res,
             _ = time::sleep(max_export_timeout) => Err(OTelSdkError::Timeout(max_export_timeout)),
        }
    }

    async fn run(mut self, mut message_receiver: mpsc::Receiver<BatchMessage>) {
        // Timer to trigger periodic cleanup of expired traces
        let mut sweep_ticker = time::interval(self.config.sweep_interval);
        // Timer to trigger periodic export of ready traces (even if batch isn't full)
        let mut batch_ticker = time::interval(self.config.scheduled_delay);

        // If the system is under load and we miss a tick, we should skip it.
        // Trying to catch up would only increase the load on the Router.
        sweep_ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        batch_ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        // The main event loop that drives the processor.
        loop {
            select! {
                // Clean up finished export tasks.
                // This removes completed tasks from the `JoinSet` to free memory.
                // The guard prevents a busy loop when there are no tasks running.
                _ = self.export_tasks.join_next(), if !self.export_tasks.is_empty() => {},

                // Periodic cleanup (sweep).
                _ = sweep_ticker.tick() => {
                    self.cached_time = Instant::now();
                    self.sweep();
                    // If sweeping produced enough traces, export immediately
                    if self.export_queue.len() >= self.config.max_export_batch_size {
                        self.flush_export_queue().await;
                    }
                },
                // Periodic batch flushing.
                _ = batch_ticker.tick() => {
                    self.cached_time = Instant::now();
                    // Export whatever queued traces we have pending
                    if !self.export_queue.is_empty() {
                        self.flush_export_queue().await;
                    }
                },
                // Processing incoming messages.
                message = message_receiver.recv() => {
                    self.cached_time = Instant::now();
                    match message {
                        Some(message) => {
                            if !self.process_message(message).await {
                                break;
                            }
                        },
                        None => break,
                    }
                },
            }
        }
    }
    /// Adds a span to the corresponding `ActiveTrace`.
    ///
    /// If the trace doesn't exist, it creates a new one (unless memory limit is hit).
    /// If memory limit is hit, it tries to sweep old traces first.
    fn handle_span(&mut self, span: SpanData) {
        let trace_id = span.span_context.trace_id();
        let now = self.cached_time;

        // In case of active trace, add the span and check if the root span has ended.
        if let Some(trace) = self.active_traces.get_mut(&trace_id) {
            let root_ended_now = trace.add_span(span, now, &self.config, &self.drop_metrics);
            if root_ended_now {
                self.enqueue_completed(trace_id);
            }
            // It's still active, so no need to do anything else
            return;
        }

        // Check Memory Limit
        if self.active_traces.len() >= self.config.max_traces_in_memory {
            // Try to free space
            self.sweep();

            // If still full, we have to drop to protect memory
            if self.active_traces.len() >= self.config.max_traces_in_memory {
                self.drop_metrics
                    .dropped_spans_count
                    .fetch_add(1, Ordering::Relaxed);
                let max_traces_in_memory = self.config.max_traces_in_memory;
                warn!(
                    name: "TraceBatchingProcessor.SpanDiscarded",
                    trace_id = %span.span_context.trace_id(),
                    %max_traces_in_memory,
                    "Memory limit reached, dropping span"
                );
                return;
            }
        }

        let mut trace = ActiveTrace::new(
            now,
            // Preallocates memory for min of 64 spans
            std::cmp::min(self.config.max_spans_per_trace, 64),
        );
        let root_ended_now = trace.add_span(span, now, &self.config, &self.drop_metrics);

        if root_ended_now {
            self.enqueue_for_export(trace);
            return;
        }

        self.active_traces.insert(trace_id, trace);
    }

    /// Scans all active traces to find:
    /// 1. Traces that finished (root ended) and are ready to export.
    /// 2. Traces that are too old, to drop them.
    fn sweep(&mut self) {
        let now = self.cached_time;
        let mut to_export = Vec::new();
        let mut to_drop = Vec::new();

        // Identify traces ready to be exported
        for (trace_id, state) in &self.active_traces {
            if state.is_complete() {
                // Trace finished normally
                to_export.push(*trace_id);
                continue;
            }

            if state.lifetime_exceeded(now, self.config.max_trace_lifetime) {
                // Downstream exporter will drop this anyway, so kill it here.
                // Why? HiveConsoleExporter is the end of the pipeline, and drops unfinished traces.
                to_drop.push(*trace_id);
            }
        }

        // Drop traces that are too old
        for trace_id in to_drop {
            if let Some(trace) = self.active_traces.remove(&trace_id) {
                debug!(name: "TraceBatchingProcessor.TraceDiscarded", %trace_id, "Trace expired without root end");
                self.drop_metrics
                    .dropped_spans_count
                    .fetch_add(trace.spans.len(), Ordering::Relaxed);
            }
        }

        // Move them to a queue of traces ready to be exported
        for trace_id in to_export {
            self.enqueue_completed(trace_id);
        }
    }

    fn drain_active_traces(&mut self) {
        let export_queue = &mut self.export_queue;
        let drop_metrics = &self.drop_metrics;
        for (_, trace) in self.active_traces.drain() {
            if trace.is_empty() {
                continue;
            }

            if trace.is_complete() {
                export_queue.push(trace);
                continue;
            }

            drop_metrics
                .dropped_spans_count
                .fetch_add(trace.spans.len(), Ordering::Relaxed);
        }
    }

    async fn flush_export_queue(&mut self) {
        while !self.export_queue.is_empty() {
            // Respect the max_export_batch_size limit
            let batch_len =
                std::cmp::min(self.export_queue.len(), self.config.max_export_batch_size);

            let start_index = self.export_queue.len() - batch_len;
            let total_spans: usize = self.export_queue[start_index..]
                .iter()
                .map(|t| t.spans.len())
                .sum();
            let mut batch = Vec::with_capacity(total_spans);

            // Pop from the end to avoid O(n) shifts when draining the front.
            for _ in 0..batch_len {
                if let Some(mut state) = self.export_queue.pop() {
                    batch.append(&mut state.spans);
                }
            }

            // If `max_concurrent_exports` is reached, it waits for a task to finish
            // before spawning a new one (backpressure).
            while !self.export_tasks.is_empty()
                && self.export_tasks.len() >= self.config.max_concurrent_exports
            {
                self.export_tasks.join_next().await;
            }

            let exporter = self.exporter.clone();
            let max_export_timeout = self.config.max_export_timeout;

            let task = async move {
                if let Err(err) = Self::export(batch, exporter, max_export_timeout).await {
                    error!(name: "TraceBatchSpanProcessor.Export.Error", reason = format!("{}", err));
                }
                Ok(())
            };

            if self.config.max_concurrent_exports == 1 {
                let _ = task.await;
            } else {
                self.export_tasks.spawn(task);
            }
        }
    }

    /// Waits for all currently running export tasks to finish.
    async fn wait_for_exports(&mut self) {
        while self.export_tasks.join_next().await.is_some() {}
    }
}

impl TraceBatchSpanProcessor {
    pub fn new<E>(exporter: E, config: &TraceBatchProcessorConfig) -> Result<Self, TelemetryError>
    where
        E: SpanExporter + Send + Sync + 'static,
    {
        if config.max_traces_in_memory == 0 {
            return Err(TelemetryError::TracesExporterSetup(
                "'max_traces_in_memory' must be greater than 0".to_string(),
            ));
        }

        let (message_sender, message_receiver) = mpsc::channel(config.max_queue_size as usize);

        let exporter = Arc::new(RwLock::new(exporter));
        let drop_metrics = Arc::new(DropMetrics::default());

        let config = Config {
            max_trace_lifetime: Duration::from_secs(60),
            sweep_interval: Duration::from_millis(200),
            max_traces_in_memory: config.max_traces_in_memory as usize,
            max_spans_per_trace: config.max_spans_per_trace as usize,
            max_export_timeout: config.max_export_timeout,
            max_export_batch_size: config.max_export_batch_size as usize,
            scheduled_delay: config.scheduled_delay,
            max_concurrent_exports: config.max_concurrent_exports as usize,
        };

        let aggregator = TraceAggregator {
            exporter,
            export_tasks: JoinSet::new(),
            config: config.clone(),
            active_traces: AHashMap::new(),
            export_queue: Vec::new(),
            cached_time: Instant::now(),
            drop_metrics: drop_metrics.clone(),
        };

        // Spawns the background worker on a dedicated OS thread with its own
        // current-thread Tokio runtime to avoid deadlocks when callers block
        // during shutdown/flush on single-thread runtimes.
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();

            match runtime {
                Ok(runtime) => {
                    runtime.block_on(async move {
                        aggregator.run(message_receiver).await;
                    });
                }
                Err(err) => {
                    eprintln!(
                        "TraceBatchSpanProcessor.Runtime.Error: failed to create Tokio runtime for trace batch processor: {}",
                        err
                    );
                }
            }
        });

        Ok(Self {
            message_sender,
            drop_metrics,
            config,
        })
    }
}
