use tracing::{field::Empty, info_span, Span};

use crate::telemetry::traces::spans::{kind::HiveSpanKind, TARGET_NAME};

#[derive(Clone)]
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
        self.span.record("cache.hit", hit);
    }
}

#[derive(Clone)]
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
        self.span.record("cache.hit", hit);
    }
}

#[derive(Clone)]
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

#[derive(Clone)]
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
        self.span.record("cache.hit", hit);
    }
}

#[derive(Clone)]
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
        self.span.record("graphql.operation.type", operation_type);
    }
}

#[derive(Clone)]
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
        self.span.record("cache.hit", hit);
    }
}

#[derive(Clone)]
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

#[derive(Clone)]
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
        );
        GraphQLOperationSpan { span }
    }

    pub fn record_document(&self, document: &str) {
        self.span.record("graphql.document.text", document);
    }
}

#[derive(Clone)]
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
        let kind: &'static str = HiveSpanKind::GraphQLSubgraphOperation.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL Subgraph Operation",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Client",
            "error.type" = Empty,
            "hive.subgraph.name" = subgraph_name,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.document.hash" = Empty,
        );
        GraphQLSubgraphOperationSpan { span }
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
            self.span().record("graphql.operation.name", name);
        }
        self.span()
            .record("graphql.operation.type", identity.operation_type);
        self.span()
            .record("graphql.document.hash", identity.client_document_hash);
        // if let Some(id) = &identity.document_id {
        //     self.span().record("graphql.operation.id", id.as_str());
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
