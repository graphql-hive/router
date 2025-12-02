use tracing::{field::Empty, info_span, Span};

use crate::traces::spans::{kind::HiveSpanKind, TARGET_NAME};

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
            "router.parse",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Server",
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
            "router.validate",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Server",
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
            "router.normalize",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Server",
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
            "router.plan",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Server",
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
            "router.authorize",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Server",
            "error.type" = Empty,
        );
        GraphQLAuthorizeSpan { span }
    }
}
