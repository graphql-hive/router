use super::compatibility::HttpCompatibilityExporter;
use super::spans::{attributes, kind::HiveSpanKind};
use hive_router_config::telemetry::tracing::SpansSemanticConventionsMode;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use std::fmt::Debug;
use std::str::FromStr;

/// Exporter wrapper for the standard (non-Hive) pipeline.
///
/// It applies semconv compatibility and pipeline-level redactions/filters
/// before forwarding to the actual exporter.
#[derive(Debug)]
pub struct StandardPipelineExporter<E: SpanExporter> {
    inner: HttpCompatibilityExporter<E>,
}

impl<E: SpanExporter> StandardPipelineExporter<E> {
    pub fn new(inner: E, mode: &SpansSemanticConventionsMode) -> Self {
        Self {
            inner: HttpCompatibilityExporter::new(inner, mode),
        }
    }

    fn strip_graphql_document(&self, span: &mut SpanData) {
        span.attributes
            .retain(|attr| attr.key.as_str() != attributes::GRAPHQL_DOCUMENT);
    }

    fn process_span(&self, span: &mut SpanData) {
        let kind_attr = span
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == attributes::HIVE_KIND);

        let Some(kv) = kind_attr else {
            return;
        };

        let opentelemetry::Value::String(kind_attribute_value) = &kv.value else {
            return;
        };

        let Ok(kind) = HiveSpanKind::from_str(kind_attribute_value.as_str()) else {
            return;
        };

        match kind {
            HiveSpanKind::GraphqlOperation | HiveSpanKind::GraphQLSubgraphOperation => {
                self.strip_graphql_document(span);
            }
            _ => {}
        }
    }
}

impl<E: SpanExporter> SpanExporter for StandardPipelineExporter<E> {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let processed_batch = batch
            .into_iter()
            .map(|mut span| {
                self.process_span(&mut span);
                span
            })
            .collect();

        self.inner.export(processed_batch).await
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
