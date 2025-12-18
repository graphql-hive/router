use tracing::{field::Empty, info_span, Span};

use crate::telemetry::traces::{
    disabled_span, is_tracing_enabled,
    spans::{attributes, kind::HiveSpanKind, TARGET_NAME},
};

pub struct GraphQLParseSpan {
    pub span: Span,
}

impl std::ops::Deref for GraphQLParseSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Default for GraphQLParseSpan {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQLParseSpan {
    pub fn new() -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlParse.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Document Parsing",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "cache.hit" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.operation.id" = Empty,
            "graphql.document.hash" = Empty,
        );
        GraphQLParseSpan { span }
    }

    pub fn record_cache_hit(&self, hit: bool) {
        self.span.record(attributes::CACHE_HIT, hit);
    }
}

pub struct GraphQLValidateSpan {
    pub span: Span,
}

impl std::ops::Deref for GraphQLValidateSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Default for GraphQLValidateSpan {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQLValidateSpan {
    pub fn new() -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlValidate.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Document Validation",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "cache.hit" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.operation.id" = Empty,
            "graphql.document.hash" = Empty,
        );
        GraphQLValidateSpan { span }
    }

    pub fn record_cache_hit(&self, hit: bool) {
        self.span.record(attributes::CACHE_HIT, hit);
    }
}

pub struct GraphQLVariableCoercionSpan {
    pub span: Span,
}

impl std::ops::Deref for GraphQLVariableCoercionSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Default for GraphQLVariableCoercionSpan {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQLVariableCoercionSpan {
    pub fn new() -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlVariableCoercion.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Variable Coercion",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.operation.id" = Empty,
            "graphql.document.hash" = Empty,
        );
        GraphQLVariableCoercionSpan { span }
    }
}

pub struct GraphQLNormalizeSpan {
    pub span: Span,
}

impl std::ops::Deref for GraphQLNormalizeSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Default for GraphQLNormalizeSpan {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQLNormalizeSpan {
    pub fn new() -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlNormalize.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Document Normalization",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "cache.hit" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.operation.id" = Empty,
            "graphql.document.hash" = Empty,
        );
        GraphQLNormalizeSpan { span }
    }

    pub fn record_cache_hit(&self, hit: bool) {
        self.span.record(attributes::CACHE_HIT, hit);
    }
}

pub struct GraphQLAuthorizeSpan {
    pub span: Span,
}

impl std::ops::Deref for GraphQLAuthorizeSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Default for GraphQLAuthorizeSpan {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQLAuthorizeSpan {
    pub fn new() -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlAuthorize.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Document Authorization",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.operation.id" = Empty,
            "graphql.document.hash" = Empty,
        );
        GraphQLAuthorizeSpan { span }
    }

    pub fn record_operation_type(&self, operation_type: &str) {
        self.span
            .record(attributes::GRAPHQL_OPERATION_TYPE, operation_type);
    }
}

pub struct GraphQLPlanSpan {
    pub span: Span,
}

impl std::ops::Deref for GraphQLPlanSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Default for GraphQLPlanSpan {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQLPlanSpan {
    pub fn new() -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlPlan.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Operation Planning",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "cache.hit" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.operation.id" = Empty,
            "graphql.document.hash" = Empty,
        );
        GraphQLPlanSpan { span }
    }

    pub fn record_cache_hit(&self, hit: bool) {
        self.span.record(attributes::CACHE_HIT, hit);
    }
}

pub struct GraphQLExecuteSpan {
    pub span: Span,
}

impl std::ops::Deref for GraphQLExecuteSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Default for GraphQLExecuteSpan {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQLExecuteSpan {
    pub fn new() -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlExecute.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Operation Execution",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.operation.id" = Empty,
            "graphql.document.hash" = Empty,
        );
        GraphQLExecuteSpan { span }
    }
}

pub struct GraphQLOperationSpan {
    pub span: Span,
}

impl std::ops::Deref for GraphQLOperationSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl Default for GraphQLOperationSpan {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQLOperationSpan {
    pub fn new() -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlOperation.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Operation",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Server",
            "error.type" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.operation.id" = Empty,
            "graphql.document.hash" = Empty,
            "graphql.document.text" = Empty,
            "hive.graphql.error.count" = Empty,
            "hive.graphql.error.codes" = Empty,
            "hive.client.name" = Empty,
            "hive.client.version" = Empty,
            "hive.graphql.operation.hash" = Empty,
        );
        GraphQLOperationSpan { span }
    }

    pub fn record_document(&self, document: &str) {
        self.span
            .record(attributes::GRAPHQL_DOCUMENT_TEXT, document);
    }

    pub fn record_hive_operation_hash(&self, hash: &str) {
        self.span
            .record(attributes::HIVE_GRAPHQL_OPERATION_HASH, hash);
    }

    // TODO: use it
    pub fn record_error_count(&self, count: u32) {
        self.span
            .record(attributes::HIVE_GRAPHQL_ERROR_COUNT, count);
    }

    // TODO: use it
    pub fn record_error_codes(&self, codes: &[&str]) {
        self.span
            .record(attributes::HIVE_GRAPHQL_ERROR_CODES, codes.join(","));
    }
}

pub struct GraphQLSubgraphOperationSpan {
    pub span: Span,
}

impl std::ops::Deref for GraphQLSubgraphOperationSpan {
    type Target = Span;
    fn deref(&self) -> &Self::Target {
        &self.span
    }
}

impl GraphQLSubgraphOperationSpan {
    pub fn new(subgraph_name: &str) -> Self {
        if !is_tracing_enabled() {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphQLSubgraphOperation.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Subgraph Operation",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Client",
            "error.type" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.document.hash" = Empty,
            "graphql.document.text" = Empty,
            "hive.graphql.error.count" = Empty,
            "hive.graphql.error.codes" = Empty,
            // Hive Console Attributes
            "hive.graphql.subgraph.name" = subgraph_name,
            "hive.client.name" = Empty,
            "hive.client.version" = Empty,
        );
        GraphQLSubgraphOperationSpan { span }
    }

    pub fn record_document(&self, document: &str) {
        self.span
            .record(attributes::GRAPHQL_DOCUMENT_TEXT, document);
    }

    // TODO: use it
    pub fn record_error_count(&self, count: u32) {
        self.span
            .record(attributes::HIVE_GRAPHQL_ERROR_COUNT, count);
    }

    // TODO: use it
    pub fn record_error_codes(&self, codes: &[&str]) {
        self.span
            .record(attributes::HIVE_GRAPHQL_ERROR_CODES, codes.join(","));
    }
}

pub struct GraphQLSpanOperationIdentity<'a> {
    pub name: Option<&'a str>,
    pub operation_type: &'a str,
    /// Hash of the original document sent to the router, by the client.
    pub client_document_hash: &'a str,
}

pub trait RecordOperationIdentity {
    fn span(&self) -> &Span;

    fn record_operation_identity(&self, identity: GraphQLSpanOperationIdentity) {
        if let Some(name) = &identity.name {
            self.span().record(attributes::GRAPHQL_OPERATION_NAME, name);
        }
        self.span()
            .record(attributes::GRAPHQL_OPERATION_TYPE, identity.operation_type);
        self.span().record(
            attributes::GRAPHQL_DOCUMENT_HASH,
            identity.client_document_hash,
        );
        // if let Some(id) = &identity.document_id {
        //     self.span().record(attributes::GRAPHQL_OPERATION_ID, id.as_str());
        // }
    }
}

// Implement RecordOperationIdentity for all span types, using a macro
// to reduce boilerplate.
macro_rules! impl_record_operation_identity {
    ($($span_type:ty),*) => {
        $(
            impl RecordOperationIdentity for $span_type {
                fn span(&self) -> &Span {
                    &self.span
                }
            }
        )*
    };
}

impl_record_operation_identity!(
    GraphQLOperationSpan,
    GraphQLParseSpan,
    GraphQLValidateSpan,
    GraphQLVariableCoercionSpan,
    GraphQLNormalizeSpan,
    GraphQLAuthorizeSpan,
    GraphQLPlanSpan,
    GraphQLExecuteSpan,
    GraphQLSubgraphOperationSpan
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::traces::spans::attributes;

    fn assert_fields(span: &Span, expected_fields: &[&str]) {
        for field in expected_fields {
            assert!(
                span.field(*field).is_some(),
                "Field '{}' is missing from span '{}'",
                field,
                span.metadata().expect("Span should have metadata").name()
            );
        }
    }

    #[test]
    fn test_graphql_parse_span_fields() {
        let span = GraphQLParseSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::CACHE_HIT,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_OPERATION_ID,
                attributes::GRAPHQL_DOCUMENT_HASH,
            ],
        );
    }

    #[test]
    fn test_graphql_validate_span_fields() {
        let span = GraphQLValidateSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::CACHE_HIT,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_OPERATION_ID,
                attributes::GRAPHQL_DOCUMENT_HASH,
            ],
        );
    }

    #[test]
    fn test_graphql_variable_coercion_span_fields() {
        let span = GraphQLVariableCoercionSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_OPERATION_ID,
                attributes::GRAPHQL_DOCUMENT_HASH,
            ],
        );
    }

    #[test]
    fn test_graphql_normalize_span_fields() {
        let span = GraphQLNormalizeSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::CACHE_HIT,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_OPERATION_ID,
                attributes::GRAPHQL_DOCUMENT_HASH,
            ],
        );
    }

    #[test]
    fn test_graphql_authorize_span_fields() {
        let span = GraphQLAuthorizeSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_OPERATION_ID,
                attributes::GRAPHQL_DOCUMENT_HASH,
            ],
        );
    }

    #[test]
    fn test_graphql_plan_span_fields() {
        let span = GraphQLPlanSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::CACHE_HIT,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_OPERATION_ID,
                attributes::GRAPHQL_DOCUMENT_HASH,
            ],
        );
    }

    #[test]
    fn test_graphql_execute_span_fields() {
        let span = GraphQLExecuteSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_OPERATION_ID,
                attributes::GRAPHQL_DOCUMENT_HASH,
            ],
        );
    }

    #[test]
    fn test_graphql_operation_span_fields() {
        let span = GraphQLOperationSpan::new();
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_OPERATION_ID,
                attributes::GRAPHQL_DOCUMENT_HASH,
                attributes::GRAPHQL_DOCUMENT_TEXT,
                attributes::HIVE_GRAPHQL_ERROR_COUNT,
                attributes::HIVE_GRAPHQL_ERROR_CODES,
                attributes::HIVE_CLIENT_NAME,
                attributes::HIVE_CLIENT_VERSION,
                attributes::HIVE_GRAPHQL_OPERATION_HASH,
            ],
        );
    }

    #[test]
    fn test_graphql_subgraph_operation_span_fields() {
        let span = GraphQLSubgraphOperationSpan::new("test-subgraph");
        assert_fields(
            &span,
            &[
                attributes::HIVE_KIND,
                attributes::OTEL_STATUS_CODE,
                attributes::OTEL_KIND,
                attributes::ERROR_TYPE,
                attributes::GRAPHQL_OPERATION_NAME,
                attributes::GRAPHQL_OPERATION_TYPE,
                attributes::GRAPHQL_DOCUMENT_HASH,
                attributes::GRAPHQL_DOCUMENT_TEXT,
                attributes::HIVE_GRAPHQL_ERROR_COUNT,
                attributes::HIVE_GRAPHQL_ERROR_CODES,
                attributes::HIVE_GRAPHQL_SUBGRAPH_NAME,
                attributes::HIVE_CLIENT_NAME,
                attributes::HIVE_CLIENT_VERSION,
            ],
        );
    }
}
