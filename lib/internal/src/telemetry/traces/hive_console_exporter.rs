//! Hive Console Exporter
//!
//! The exporter receives spans with original attributes registered at compile time in `http_request.rs` file.
//!
//! Root Span requirements:
//!  - attributes:
//!     - http.status_code
//!     - http.host
//!     - http.method
//!     - http.route
//!     - http.url
//!     - hive.client.name
//!     - hive.client.version
//!     - graphql.operation.name
//!     - graphql.operation.type
//!     - graphql.document
//!     - hive.graphql.operation.hash (same as for usage reporting)
//!     - hive.graphql.error.count
//!     - hive.graphql.error.codes
//!     - hive.gateway.operation.subgraph.names (it could be converted into hive.graphql.subgraph.names)
//!     - hive.graphql (no idea what the value should be)  (indication of the subgraph span)
//!  - no parent span  (indication of the subgraph span)
//!
//! Subgraph Span requirements:
//!  - attributes:
//!     - hive.graphql.subgraph.name (indication of the subgraph span)
//!     - http.status_code
//!     - http.host
//!     - http.method
//!     - http.route (weird an http.client has it)
//!     - http.url
//!     - hive.client.name (weird to have outside of root span)
//!     - hive.client.version (weird to have outside of root span)
//!     - graphql.operation.name - is it the name of the operations executed against the subgraph?
//!     - graphql.operation.type
//!     - graphql.document
//!     - hive.graphql.error.count
//!     - hive.graphql.error.code
//!

use opentelemetry::trace::SpanId;
use opentelemetry::KeyValue;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use crate::telemetry::traces::spans::kind::HiveSpanKind;

const HTTP_SERVER_TO_GRAPHQL_ATTR_MAP: &[(&str, &str)] = &[
    ("http.response.status_code", "http.status_code"),
    ("server.address", "http.host"),
    ("http.request.method", "http.method"),
    ("http.route", "http.route"),
    ("url.full", "http.url"),
];

const HTTP_CLIENT_TO_GRAPHQL_ATTR_MAP: &[(&str, &str)] = &[
    ("http.response.status_code", "http.status_code"),
    ("server.address", "http.host"),
    ("http.request.method", "http.method"),
    ("url.path", "http.route"),
    ("url.full", "http.url"),
];

const GRAPHQL_TO_HIVE_GRAPHQL_OPERATION_ATTR_MAP: &[(&str, &str)] =
    &[("graphql.document.text", "graphql.document")];

#[derive(Debug)]
pub struct HiveConsoleExporter<E: SpanExporter> {
    inner: E,
}

impl<E: SpanExporter> HiveConsoleExporter<E> {
    pub fn new(inner: E) -> Self {
        Self { inner }
    }

    #[inline]
    fn process_spans(&self, batch: &mut Vec<SpanData>) {
        if batch.is_empty() {
            return;
        }

        // TODO: Let's make attribute names as constants,
        //       and add a test for every span
        //       to ensure all attributes are present.
        //       We need attribute names as constants,
        //       to avoid typos and make refactoring easier.
        //       We can't use constants in span_info macro,
        //       so the only way to verify is to add tests.

        let mut http_server_indexes: Vec<usize> = Vec::new();
        let mut graphql_root_operation_indexes: Vec<usize> = Vec::new();
        let mut graphql_subgraph_operation_indexes: Vec<usize> = Vec::new();
        let mut http_client_or_inflight_indexes: Vec<usize> = Vec::new();

        for (index, span) in batch.iter().enumerate() {
            let Some(kind) = self.get_hive_kind(span) else {
                continue;
            };

            match kind {
                HiveSpanKind::HttpServerRequest => {
                    http_server_indexes.push(index);
                }
                HiveSpanKind::GraphqlOperation => {
                    graphql_root_operation_indexes.push(index);
                }
                HiveSpanKind::GraphQLSubgraphOperation => {
                    graphql_subgraph_operation_indexes.push(index);
                }
                HiveSpanKind::HttpClientRequest | HiveSpanKind::HttpInflightRequest => {
                    http_client_or_inflight_indexes.push(index);
                }
                _ => {
                    // Leave other spans untouched
                }
            }
        }

        if http_server_indexes.is_empty() {
            return;
        }

        // Transfer root span attributes (http.server -> graphql.operation)
        self.http_server_attr_to_graphql_operation(batch, &http_server_indexes);
        // Transfer subgraph span attributes (http.client/http.inflight -> graphql.subgraph.operation)
        self.http_client_attr_to_subgraph_graphql_operation(
            batch,
            &graphql_subgraph_operation_indexes,
            &http_client_or_inflight_indexes,
        );
        // Gather subgraph names into hive.gateway.operation.subgraph.names attribute
        self.subgraph_attr_to_root(
            batch,
            &graphql_subgraph_operation_indexes,
            &graphql_root_operation_indexes,
        );
        // Remove all http.server spans (in reverse order to avoid index shifting)
        let mut indices_to_remove = http_server_indexes;
        indices_to_remove.sort_unstable_by(|a, b| b.cmp(a));
        for idx in indices_to_remove {
            batch.swap_remove(idx);
        }
        // Add hive.graphql=true to all spans so Hive Console accepts them
        self.add_hive_graphql_flag(batch);
    }

    #[inline]
    fn http_server_attr_to_graphql_operation(
        &self,
        batch: &mut [SpanData],
        http_server_indexes: &[usize],
    ) {
        for http_idx in http_server_indexes {
            let http_idx = *http_idx;

            // Find the graphql.operation span that has this http.server as parent
            let Some(graphql_idx) = batch
                .iter()
                .position(|span| span.parent_span_id == batch[http_idx].span_context.span_id())
            else {
                // TODO: add tracing::error
                continue;
            };

            // Hive Console expects no parent span for root GraphQL operation spans
            batch[graphql_idx].parent_span_id = SpanId::INVALID;

            if http_idx == graphql_idx {
                tracing::error!(
                    { component = "hive_console_exporter" },
                    "Expected 'http.server' and 'graphql.operation' to be different spans. Trace '{}' ignored.",
                    batch[http_idx].span_context.trace_id()
                );
                continue;
            }

            if http_idx > batch.len() || graphql_idx > batch.len() {
                tracing::error!(
                    { component = "hive_console_exporter" },
                    "Span indexes out of bounds. Trace '{}' ignored.",
                    batch[http_idx].span_context.trace_id()
                );
                continue;
            }

            let (http_span, graphql_span) = if http_idx < graphql_idx {
                let (before, after) = batch.split_at_mut(graphql_idx);
                (&mut before[http_idx], &mut after[0])
            } else {
                let (before, after) = batch.split_at_mut(http_idx);
                (&mut after[0], &mut before[graphql_idx])
            };

            // Transfer mapped attributes
            for (http_attr_key, graphql_attr_key) in HTTP_SERVER_TO_GRAPHQL_ATTR_MAP {
                let Some(idx) = http_span
                    .attributes
                    .iter()
                    .position(|kv| kv.key.as_str() == *http_attr_key)
                else {
                    tracing::debug!(
                        { component = "hive_console_exporter" },
                        "Attribute '{}' not found in 'http.server' span. Ignoring.",
                        http_attr_key
                    );
                    continue;
                };

                let value = http_span.attributes.swap_remove(idx).value;
                graphql_span
                    .attributes
                    .push(KeyValue::new(*graphql_attr_key, value));
            }

            for (existing_key, replacing_key) in GRAPHQL_TO_HIVE_GRAPHQL_OPERATION_ATTR_MAP {
                let Some(idx) = graphql_span
                    .attributes
                    .iter()
                    .position(|kv| kv.key.as_str() == *existing_key)
                else {
                    tracing::debug!(
                        { component = "hive_console_exporter" },
                        "Attribute '{}' not found in 'graphql.operation' span. Ignoring.",
                        existing_key
                    );
                    continue;
                };

                let value = graphql_span.attributes.swap_remove(idx).value;
                graphql_span
                    .attributes
                    .push(KeyValue::new(*replacing_key, value));
            }
        }
    }

    #[inline]
    fn http_client_attr_to_subgraph_graphql_operation(
        &self,
        batch: &mut [SpanData],
        graphql_subgraph_operation_indexes: &[usize],
        http_client_or_inflight_indexes: &[usize],
    ) {
        for graphql_idx in graphql_subgraph_operation_indexes {
            let graphql_idx = *graphql_idx;
            let graphql_span_id = batch[graphql_idx].span_context.span_id();
            let Some(http_idx) = http_client_or_inflight_indexes.iter().find_map(|http_idx| {
                let http_idx = *http_idx;

                if batch[http_idx].parent_span_id == graphql_span_id {
                    Some(http_idx)
                } else {
                    None
                }
            }) else {
                tracing::error!(
                    { component = "hive_console_exporter" },
                    "No matching 'http.client' or 'http.inflight' span found for 'graphql.subgraph.operation' span. Trace '{}' ignored.",
                    batch[graphql_idx].span_context.trace_id()
                );
                continue;
            };

            let (http_span, graphql_span) = if http_idx < graphql_idx {
                let (before, after) = batch.split_at_mut(graphql_idx);
                (&mut before[http_idx], &mut after[0])
            } else {
                let (before, after) = batch.split_at_mut(http_idx);
                (&mut after[0], &mut before[graphql_idx])
            };

            // Transfer mapped attributes
            for (http_attr_key, graphql_attr_key) in HTTP_CLIENT_TO_GRAPHQL_ATTR_MAP {
                let Some(idx) = http_span
                    .attributes
                    .iter()
                    .position(|kv| kv.key.as_str() == *http_attr_key)
                else {
                    tracing::warn!(
                        { component = "hive_console_exporter" },
                        "Attribute '{}' not found in 'http.client' span. Ignoring.",
                        http_attr_key
                    );
                    continue;
                };

                let value = http_span.attributes.swap_remove(idx).value;
                graphql_span
                    .attributes
                    .push(KeyValue::new(*graphql_attr_key, value));
            }

            for (existing_key, replacing_key) in GRAPHQL_TO_HIVE_GRAPHQL_OPERATION_ATTR_MAP {
                let Some(idx) = graphql_span
                    .attributes
                    .iter()
                    .position(|kv| kv.key.as_str() == *existing_key)
                else {
                    tracing::warn!(
                        { component = "hive_console_exporter" },
                        "Attribute '{}' not found in 'graphql.operation' span. Ignoring.",
                        existing_key
                    );
                    continue;
                };

                let value = graphql_span.attributes.swap_remove(idx).value;
                graphql_span
                    .attributes
                    .push(KeyValue::new(*replacing_key, value));
            }
        }
    }

    #[inline]
    fn subgraph_attr_to_root(
        &self,
        batch: &mut [SpanData],
        graphql_subgraph_operation_indexes: &[usize],
        graphql_root_operation_indexes: &[usize],
    ) {
        // I need to collect all subgraph names from subgraph spans and add them to the root span
        let mut trace_id_to_subgraph_names: std::collections::HashMap<
            opentelemetry::TraceId,
            HashSet<String>,
        > = HashMap::new();

        for graphql_idx in graphql_subgraph_operation_indexes {
            let graphql_idx = *graphql_idx;
            let trace_id = batch[graphql_idx].span_context.trace_id();

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

            trace_id_to_subgraph_names
                .entry(trace_id)
                .or_default()
                .insert(subgraph_name.to_string());
        }

        for root_idx in graphql_root_operation_indexes {
            let root_idx = *root_idx;
            let trace_id = batch[root_idx].span_context.trace_id();
            let Some(subgraph_names) = trace_id_to_subgraph_names.get(&trace_id) else {
                continue;
            };
            batch[root_idx].attributes.push(KeyValue::new(
                "hive.gateway.operation.subgraph.names",
                subgraph_names
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ));
        }
    }

    #[inline]
    fn get_hive_kind<'a>(&self, span: &'a SpanData) -> Option<HiveSpanKind> {
        span.attributes
            .iter()
            .find(|kv| kv.key.as_str() == "hive.kind")
            .and_then(|kv| match &kv.value {
                opentelemetry::Value::String(s) => {
                    Some(HiveSpanKind::from_str(s.as_str()).expect("Invalid hive.kind value"))
                }
                _ => None,
            })
    }

    /// Add hive.graphql=true to all spans
    #[inline]
    fn add_hive_graphql_flag(&self, batch: &mut [SpanData]) {
        for span in batch.iter_mut() {
            span.attributes.push(KeyValue::new("hive.graphql", true));
        }
    }
}

impl<E: SpanExporter> SpanExporter for HiveConsoleExporter<E> {
    async fn export(&self, mut batch: Vec<SpanData>) -> OTelSdkResult {
        self.process_spans(&mut batch);
        self.inner.export(batch).await
    }

    fn shutdown(&mut self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn force_flush(&mut self) -> OTelSdkResult {
        self.inner.force_flush()
    }
}
