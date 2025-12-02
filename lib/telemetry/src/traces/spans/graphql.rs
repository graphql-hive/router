use tracing::{field::Empty, info_span, Span};

use crate::traces::spans::{kind::HiveSpanKind, TARGET_NAME};

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
            "graphql.document" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
        );
        GraphQLOperationSpan { span }
    }

    pub fn record_document(&self, document: &str) {
        self.span.record("graphql.document", document);
    }

    pub fn record_operation_name(&self, operation_name: &str) {
        self.span.record("graphql.operation.name", operation_name);
    }

    pub fn record_operation_type(&self, operation_type: &str) {
        self.span.record("graphql.operation.type", operation_type);
    }
}

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

impl GraphQLParseSpan {
    pub fn new() -> Self {
        let kind: &'static str = HiveSpanKind::GraphqlParse.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL - Parse",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "cache.hit" = Empty,
        );
        GraphQLParseSpan { span }
    }

    pub fn record_cache_hit(&self, hit: bool) {
        self.span.record("cache.hit", &hit);
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

impl GraphQLValidateSpan {
    pub fn new() -> Self {
        let kind: &'static str = HiveSpanKind::GraphqlValidate.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL - Validate",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "cache.hit" = Empty,
        );
        GraphQLValidateSpan { span }
    }

    pub fn record_cache_hit(&self, hit: bool) {
        self.span.record("cache.hit", &hit);
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

impl GraphQLNormalizeSpan {
    pub fn new() -> Self {
        let kind: &'static str = HiveSpanKind::GraphqlNormalize.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL - Normalize",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "cache.hit" = Empty,
        );
        GraphQLNormalizeSpan { span }
    }

    pub fn record_cache_hit(&self, hit: bool) {
        self.span.record("cache.hit", &hit);
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

impl GraphQLPlanSpan {
    pub fn new() -> Self {
        let kind: &'static str = HiveSpanKind::GraphqlPlan.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL - Plan",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "cache.hit" = Empty,
        );
        GraphQLPlanSpan { span }
    }

    pub fn record_cache_hit(&self, hit: bool) {
        self.span.record("cache.hit", &hit);
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

impl GraphQLAuthorizeSpan {
    pub fn new() -> Self {
        let kind: &'static str = HiveSpanKind::GraphqlAuthorize.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL - Authorize",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
        );
        GraphQLAuthorizeSpan { span }
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

impl GraphQLExecuteSpan {
    pub fn new() -> Self {
        let kind: &'static str = HiveSpanKind::GraphqlExecute.into();
        let span = info_span!(
            target: TARGET_NAME,
            "GraphQL - Execute",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Internal",
            "error.type" = Empty,
        );
        GraphQLExecuteSpan { span }
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
            "otel.kind" = "Internal",
            "error.type" = Empty,
            "hive.subgraph.name" = subgraph_name
        );
        GraphQLSubgraphOperationSpan { span }
    }
}
