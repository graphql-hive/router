//! Hive Console exporter span normalization.
//!
//! The Hive Console expects GraphQL spans to have HTTP metadata and to be
//! rooted at `graphql.operation` rather than `http.server`.
//! This exporter rewrites batches to fit those requirements.
//!
//! - Promotes each `graphql.operation` span to be the trace root (no parent).
//! - Moves HTTP server attributes to the root `graphql.operation` span.
//! - Moves HTTP client/inflight attributes to its parent `graphql.subgraph.operation` spans.
//! - Renames deprecated GraphQL attributes to Hive-specific keys where needed.
//! - Copies timing from the HTTP server span to the root `graphql.operation` span.
//! - Adds `hive.graphql=true` and a comma-joined list of subgraph names on the root span.
//! - Drop the original `http.server` spans after metadata is transferred.
//!
//! ## Trace acceptence rules:
//! - If a batch contains no `http.server` spans, it is cleared as it's missing root spans.
//! - If an `http.server` span has no `graphql.operation` child, the whole trace is excluded,
//!   as it's an incomplete trace and we can't produce something accepted by Hive Console.
//! - If a `graphql.subgraph.operation` span has no `http.client`/`http.inflight` child,
//!   the whole trace is excluded, for similar reasons as above.
//! - Excluded traces are removed from the batch at the end of processing.
//!
//! ## Span relationship:
//!   http.server -> graphql.operation -> graphql.subgraph.operation -> http.client / http.inflight
//!
//! ## Batch flow:
//!   build index
//!      v
//!   normalize root (http.server -> graphql.operation)
//!      v
//!   normalize subgraph (http.client -> graphql.subgraph.operation)
//!      v
//!   add subgraph names to root
//!      v
//!   drop http.server spans
//!      v
//!   drop ignored traces
//!
use ahash::{HashMap, HashSet};
use opentelemetry::trace::SpanId;
use opentelemetry::{KeyValue, TraceId};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};

use crate::telemetry::traces::spans::{attributes, kind::HiveSpanKind};

/// Map of attributes to be renamed when normalizing http.server spans to graphql.operation spans.
const HTTP_SERVER_TO_GRAPHQL_ATTR_MAP: &[(&str, &str)] = &[
    (
        attributes::HTTP_RESPONSE_STATUS_CODE,
        attributes::DEPRECATED_HTTP_STATUS_CODE,
    ),
    (attributes::SERVER_ADDRESS, attributes::DEPRECATED_HTTP_HOST),
    (
        attributes::HTTP_REQUEST_METHOD,
        attributes::DEPRECATED_HTTP_METHOD,
    ),
    (attributes::HTTP_ROUTE, attributes::HTTP_ROUTE),
    (attributes::URL_FULL, attributes::DEPRECATED_HTTP_URL),
];

/// Map of attributes to be renamed when normalizing http.client spans to graphql.subgraph.operation spans.
const HTTP_CLIENT_TO_GRAPHQL_ATTR_MAP: &[(&str, &str)] = &[
    (
        attributes::HTTP_RESPONSE_STATUS_CODE,
        attributes::DEPRECATED_HTTP_STATUS_CODE,
    ),
    (attributes::SERVER_ADDRESS, attributes::DEPRECATED_HTTP_HOST),
    (
        attributes::HTTP_REQUEST_METHOD,
        attributes::DEPRECATED_HTTP_METHOD,
    ),
    (attributes::URL_PATH, attributes::HTTP_ROUTE),
    (attributes::URL_FULL, attributes::DEPRECATED_HTTP_URL),
];

const GRAPHQL_TO_HIVE_OPERATION_ATTR_RENAMES: &[(&str, &str)] = &[(
    attributes::GRAPHQL_DOCUMENT_TEXT,
    attributes::DEPRECATED_GRAPHQL_DOCUMENT,
)];

struct SpanBatchContext {
    /// Trace IDs to drop
    ignored_trace_ids: HashSet<TraceId>,
    /// Indices of http.server spans in the batch.
    http_server_span_indices: Vec<usize>,
    /// Indices of graphql.operation spans in the batch.
    root_graphql_span_indices: Vec<usize>,
    /// Indices of graphql.subgraph.operation spans in the batch.
    subgraph_graphql_span_indices: Vec<usize>,
    /// SpanId -> batch index lookup for parent/child resolution.
    span_id_to_index: HashMap<SpanId, usize>,
    /// Children grouped by parent span index (parent_idx -> [child_idx...]).
    children_by_parent_index: Vec<Vec<usize>>,
    /// Parsed span kind per batch index. None if not recognized or not relevant.
    kind_by_index: Vec<Option<HiveSpanKind>>,
}

impl SpanBatchContext {
    fn new(batch_size: usize) -> Self {
        Self {
            ignored_trace_ids: HashSet::default(),
            http_server_span_indices: Vec::new(),
            root_graphql_span_indices: Vec::new(),
            subgraph_graphql_span_indices: Vec::new(),
            span_id_to_index: HashMap::default(),
            children_by_parent_index: vec![Vec::new(); batch_size],
            kind_by_index: vec![None; batch_size],
        }
    }
}

#[derive(Debug)]
pub struct HiveConsoleExporter<E: SpanExporter> {
    inner: E,
}

impl<E: SpanExporter> HiveConsoleExporter<E> {
    pub fn new(inner: E) -> Self {
        Self { inner }
    }

    fn process_spans(&self, batch: &mut Vec<SpanData>) {
        if batch.is_empty() {
            return;
        }

        let mut context = SpanBatchContext::new(batch.len());
        // Resolve span IDs to indices so we can map parents to children.
        for (index, span) in batch.iter().enumerate() {
            context
                .span_id_to_index
                .insert(span.span_context.span_id(), index);
        }

        // Categorize spans and build parent -> child index lists
        for (index, span) in batch.iter().enumerate() {
            context.kind_by_index[index] = self.get_hive_kind(span);
            let Some(kind) = &context.kind_by_index[index] else {
                continue;
            };

            // Build parent-to-child map
            if span.parent_span_id != SpanId::INVALID && !span.parent_span_is_remote {
                if let Some(&parent_index) = context.span_id_to_index.get(&span.parent_span_id) {
                    context.children_by_parent_index[parent_index].push(index);
                }
            }

            match kind {
                HiveSpanKind::HttpServerRequest => {
                    context.http_server_span_indices.push(index);
                }
                HiveSpanKind::GraphqlOperation => {
                    context.root_graphql_span_indices.push(index);
                }
                HiveSpanKind::GraphQLSubgraphOperation => {
                    context.subgraph_graphql_span_indices.push(index);
                }
                _ => {
                    // Leave other spans untouched
                }
            }
        }

        if context.http_server_span_indices.is_empty() {
            // Hive Console expects traces rooted in http.server spans
            batch.clear();
            return;
        }

        // Normalize spans to Hive Console requirements and propagate attributes.
        self.normalize_root_operation_from_http_server(batch, &mut context);
        self.normalize_subgraph_operation_from_http_client(batch, &mut context);
        self.add_subgraph_names_to_root_operation(batch, &context);

        // Remove all http.server spans (in reverse order to avoid index shifting).
        // Hive Console uses the root GraphQL span as the entry point instead.
        let mut indices_to_remove = context.http_server_span_indices;
        indices_to_remove.sort_unstable_by(|a, b| b.cmp(a));
        for idx in indices_to_remove {
            batch.swap_remove(idx);
        }

        // Remove spans of ignored traces
        batch.retain(|span| {
            !context
                .ignored_trace_ids
                .contains(&span.span_context.trace_id())
        });
    }

    /// Normalize root graphql.operation spans using the http.server span.
    ///
    /// Hive Console wants attributes from the http.server span to be
    /// attributes and timing on the graphql.operation span instead.
    fn normalize_root_operation_from_http_server(
        &self,
        batch: &mut [SpanData],
        context: &mut SpanBatchContext,
    ) {
        for http_idx in context.http_server_span_indices.iter().copied() {
            // Find the graphql.operation span that has this http.server as parent.
            // If missing, the trace is inconsistent for Hive Console, so drop it.
            let Some(graphql_idx) = context.children_by_parent_index[http_idx]
                .iter()
                .find(|&&child_idx| {
                    context.kind_by_index[child_idx] == Some(HiveSpanKind::GraphqlOperation)
                })
                .copied()
            else {
                let trace_id = batch[http_idx].span_context.trace_id();
                tracing::error!(
                    component = "hive_console_exporter",
                    trace_id = ?trace_id,
                    "No matching graphql.operation span found for http.server span"
                );
                context.ignored_trace_ids.insert(trace_id);
                continue;
            };

            // Hive Console expects no parent span for root `graphql.operation` spans.
            batch[graphql_idx].parent_span_id = SpanId::INVALID;

            let (http_span, graphql_span) = if http_idx < graphql_idx {
                let (before, after) = batch.split_at_mut(graphql_idx);
                (&mut before[http_idx], &mut after[0])
            } else {
                let (before, after) = batch.split_at_mut(http_idx);
                (&mut after[0], &mut before[graphql_idx])
            };

            // Move attributes from http.server to `graphql.operation`
            self.move_mapped_attributes(http_span, graphql_span, HTTP_SERVER_TO_GRAPHQL_ATTR_MAP);

            // Replace deprecated `graphql.operation` attributes with Hive attributes.
            self.rename_attributes_in_place(graphql_span, GRAPHQL_TO_HIVE_OPERATION_ATTR_RENAMES);

            // Add hive.graphql=true as it's required by Hive Console.
            graphql_span
                .attributes
                .push(KeyValue::new("hive.graphql", true));

            // Transfer timing of the http.server span to the graphql.operation span,
            // so that Hive Console shows correct duration for the operation.
            graphql_span.start_time = http_span.start_time;
            graphql_span.end_time = http_span.end_time;
        }
    }

    /// Normalize graphql.subgraph.operation spans using the http.client/http.inflight span.
    ///
    /// Hive Console expects the http metadata on graphql.subgraph.operation spans.
    fn normalize_subgraph_operation_from_http_client(
        &self,
        batch: &mut [SpanData],
        context: &mut SpanBatchContext,
    ) {
        for graphql_idx in context.subgraph_graphql_span_indices.iter().copied() {
            let trace_id = batch[graphql_idx].span_context.trace_id();
            if context.ignored_trace_ids.contains(&trace_id) {
                // This trace is already ignored
                continue;
            }

            let Some(http_idx) = context.children_by_parent_index[graphql_idx]
                .iter()
                .find(|&&child_idx| {
                    matches!(
                        context.kind_by_index[child_idx],
                        Some(HiveSpanKind::HttpClientRequest)
                            | Some(HiveSpanKind::HttpInflightRequest)
                    )
                })
                .copied()
            else {
                tracing::error!(
                    component = "hive_console_exporter",
                    trace_id = ?trace_id,
                    "No matching http.client or http.inflight span found for graphql.subgraph.operation"
                );
                context.ignored_trace_ids.insert(trace_id);
                continue;
            };

            let (http_span, graphql_span) = if http_idx < graphql_idx {
                let (before, after) = batch.split_at_mut(graphql_idx);
                (&mut before[http_idx], &mut after[0])
            } else {
                let (before, after) = batch.split_at_mut(http_idx);
                (&mut after[0], &mut before[graphql_idx])
            };

            // Move attributes from http.client/http.inflight to graphql.subgraph.operation
            self.move_mapped_attributes(http_span, graphql_span, HTTP_CLIENT_TO_GRAPHQL_ATTR_MAP);

            // Replace deprecated graphql attributes with Hive attributes.
            self.rename_attributes_in_place(graphql_span, GRAPHQL_TO_HIVE_OPERATION_ATTR_RENAMES);
        }
    }

    /// Move attributes from source span to target span.
    fn move_mapped_attributes(
        &self,
        source_span: &mut SpanData,
        target_span: &mut SpanData,
        attr_map: &[(&'static str, &'static str)],
    ) {
        for (source_key, target_key) in attr_map.iter().copied() {
            let Some(idx) = source_span
                .attributes
                .iter()
                .position(|kv| kv.key.as_str() == source_key)
            else {
                tracing::debug!(
                    component = "hive_console_exporter",
                    attribute_key = %source_key,
                    "Attribute not found in source span"
                );
                continue;
            };

            let kv = source_span.attributes.swap_remove(idx);
            target_span
                .attributes
                .push(KeyValue::new(target_key, kv.value));
        }
    }

    /// Rename attributes in a span from old key to new key.
    fn rename_attributes_in_place(
        &self,
        span: &mut SpanData,
        renames: &[(&'static str, &'static str)],
    ) {
        for (old_key, new_key) in renames.iter().copied() {
            let Some(idx) = span
                .attributes
                .iter()
                .position(|kv| kv.key.as_str() == old_key)
            else {
                tracing::debug!(
                    component = "hive_console_exporter",
                    attribute_key = %old_key,
                    "Attribute not found for renaming"
                );
                continue;
            };

            let kv = span.attributes.swap_remove(idx);
            span.attributes.push(KeyValue::new(new_key, kv.value));
        }
    }

    fn add_subgraph_names_to_root_operation(
        &self,
        batch: &mut [SpanData],
        context: &SpanBatchContext,
    ) {
        let mut subgraph_names_by_trace: HashMap<TraceId, Vec<&str>> = HashMap::default();

        // Collect all subgraph names from subgraph spans.
        // We aggregate per trace so the root operation can list all subgraphs used.
        for graphql_idx in context.subgraph_graphql_span_indices.iter().copied() {
            let trace_id = batch[graphql_idx].span_context.trace_id();

            if context.ignored_trace_ids.contains(&trace_id) {
                // This trace is already ignored
                continue;
            }

            let subgraph_name_opt = batch[graphql_idx]
                .attributes
                .iter()
                .find(|kv| kv.key.as_str() == "hive.graphql.subgraph.name")
                .and_then(|kv| match &kv.value {
                    opentelemetry::Value::String(s) => Some(s.as_str()),
                    _ => None,
                });

            let Some(subgraph_name) = subgraph_name_opt else {
                continue;
            };

            let names = subgraph_names_by_trace.entry(trace_id).or_default();
            if !names.contains(&subgraph_name) {
                names.push(subgraph_name);
            }
        }

        if subgraph_names_by_trace.is_empty() {
            return;
        }

        let mut root_indices_with_names: Vec<(usize, String)> = Vec::new();

        // Collect a pair of index and subgraph names string for each root operation.
        // We sort and join for deterministic output in Hive Console.
        // It will also help to reduce the cardinality of attribute's values.
        for idx in context.root_graphql_span_indices.iter().copied() {
            let trace_id = batch[idx].span_context.trace_id();
            let Some(subgraph_names) = subgraph_names_by_trace.get_mut(&trace_id) else {
                continue;
            };

            subgraph_names.sort_unstable();

            root_indices_with_names.push((idx, subgraph_names.join(",")));
        }

        // Extra loop to bypass the double mutable borrow issue.
        // We can't mutate the batch and the subgraph names map in the same loop.
        for (root_idx, subgraph_names_buffer) in root_indices_with_names {
            batch[root_idx].attributes.push(KeyValue::new(
                "hive.gateway.operation.subgraph.names",
                subgraph_names_buffer,
            ));
        }
    }

    #[inline]
    fn get_hive_kind(&self, span: &SpanData) -> Option<HiveSpanKind> {
        let name = span.name.as_ref();

        match name {
            _ if name == HiveSpanKind::GraphqlOperation.as_str() => {
                Some(HiveSpanKind::GraphqlOperation)
            }
            _ if name == HiveSpanKind::GraphQLSubgraphOperation.as_str() => {
                Some(HiveSpanKind::GraphQLSubgraphOperation)
            }
            _ if name == HiveSpanKind::HttpServerRequest.as_str() => {
                Some(HiveSpanKind::HttpServerRequest)
            }
            _ if name == HiveSpanKind::HttpClientRequest.as_str() => {
                Some(HiveSpanKind::HttpClientRequest)
            }
            _ if name == HiveSpanKind::HttpInflightRequest.as_str() => {
                Some(HiveSpanKind::HttpInflightRequest)
            }
            _ => None,
        }
    }
}

impl<E: SpanExporter> SpanExporter for HiveConsoleExporter<E> {
    async fn export(&self, mut batch: Vec<SpanData>) -> OTelSdkResult {
        self.process_spans(&mut batch);
        self.inner.export(batch).await
    }

    fn shutdown(&mut self) -> OTelSdkResult {
        tracing::info!(
            component = "telemetry",
            layer = "hive_console_exporter",
            "shutdown scheduled"
        );
        let result = self.inner.shutdown();
        tracing::info!(
            component = "telemetry",
            layer = "hive_console_exporter",
            "shutdown completed"
        );
        result
    }

    fn force_flush(&mut self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, res: &opentelemetry_sdk::Resource) {
        self.inner.set_resource(res);
    }
}
