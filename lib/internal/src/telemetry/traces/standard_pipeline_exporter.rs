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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::traces::spans::graphql::{
        GraphQLOperationSpan, GraphQLSpanOperationIdentity, GraphQLSubgraphOperationSpan,
    };
    use opentelemetry::trace::{Tracer, TracerProvider};
    use opentelemetry_sdk::trace::{
        InMemorySpanExporter, InMemorySpanExporterBuilder, SdkTracerProvider, SimpleSpanProcessor,
    };
    use tracing_subscriber::layer::SubscriberExt;

    fn find_attribute<'a>(
        span_data: &'a SpanData,
        key: &'static str,
    ) -> Option<&'a opentelemetry::Value> {
        span_data
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| &kv.value)
    }

    fn setup_test_pipeline(
        mode: SpansSemanticConventionsMode,
    ) -> (SdkTracerProvider, InMemorySpanExporter) {
        let memory_exporter = InMemorySpanExporterBuilder::new().build();
        let standard_exporter = StandardPipelineExporter::new(memory_exporter.clone(), &mode);
        let processor = SimpleSpanProcessor::new(standard_exporter);
        let provider = SdkTracerProvider::builder()
            .with_span_processor(processor)
            .build();

        (provider, memory_exporter)
    }

    fn setup_tracing_subscriber(provider: &SdkTracerProvider) -> impl Drop {
        let otel_tracer = provider.tracer("standard-pipeline");
        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(otel_tracer);
        let subscriber = tracing_subscriber::registry().with(telemetry_layer);
        tracing::subscriber::set_default(subscriber)
    }

    fn assert_graphql_document_stripped(mode: SpansSemanticConventionsMode) {
        let (provider, memory_exporter) = setup_test_pipeline(mode);
        let tracer = provider.tracer("test-tracer");
        let _guard = setup_tracing_subscriber(&provider);

        tracer.in_span("root", |_cx| {
            let operation_span = GraphQLOperationSpan::new();
            let identity = GraphQLSpanOperationIdentity {
                name: Some("GetMe"),
                operation_type: "query",
                client_document_hash: "doc-hash",
            };
            operation_span.record_details(
                "query GetMe { me }",
                identity,
                Some("client"),
                Some("1.0.0"),
                "op-hash",
            );

            let subgraph_span =
                GraphQLSubgraphOperationSpan::new("test-subgraph", "query Example { me }");
            let subgraph_identity = GraphQLSpanOperationIdentity {
                name: Some("GetMe"),
                operation_type: "query",
                client_document_hash: "hash123",
            };
            subgraph_span.record_operation_identity(subgraph_identity);
        });

        drop(_guard);
        provider.force_flush().unwrap();
        let spans = memory_exporter.get_finished_spans().unwrap();

        let operation_span = spans
            .iter()
            .find(|span| span.name.as_ref() == "graphql.operation")
            .expect("graphql.operation span");
        let subgraph_span = spans
            .iter()
            .find(|span| span.name.as_ref() == "graphql.subgraph.operation")
            .expect("graphql.subgraph.operation span");

        assert!(find_attribute(operation_span, attributes::GRAPHQL_DOCUMENT).is_none());
        assert!(find_attribute(subgraph_span, attributes::GRAPHQL_DOCUMENT).is_none());
    }

    #[test]
    fn test_graphql_document_removed_spec_compliant() {
        assert_graphql_document_stripped(SpansSemanticConventionsMode::SpecCompliant);
    }

    #[test]
    fn test_graphql_document_removed_deprecated() {
        assert_graphql_document_stripped(SpansSemanticConventionsMode::Deprecated);
    }

    #[test]
    fn test_graphql_document_removed_spec_and_deprecated() {
        assert_graphql_document_stripped(SpansSemanticConventionsMode::SpecAndDeprecated);
    }
}
