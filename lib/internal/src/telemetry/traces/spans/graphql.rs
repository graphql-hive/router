use opentelemetry::KeyValue;
use tracing::{field::Empty, info_span, record_all, Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::{
    graphql::ObservedError,
    telemetry::traces::{
        disabled_span, is_level_enabled,
        spans::{
            attributes::{
                self, ERROR_MESSAGE, ERROR_TYPE, HIVE_ERROR_AFFECTED_PATH, HIVE_ERROR_PATH,
                HIVE_ERROR_SUBGRAPH_NAME, HIVE_KIND,
            },
            kind::{HiveEventKind, HiveSpanKind},
            TARGET_NAME,
        },
    },
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
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlParse.into();
        let span = info_span!(
            target: TARGET_NAME,
            "graphql.parse",
            "hive.kind" = kind,
            "otel.kind" = "Internal",
            "cache.hit" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.document.hash" = Empty,
        );
        GraphQLParseSpan { span }
    }

    pub fn record_cache_hit(&self, hit: bool) {
        self.span.record(attributes::CACHE_HIT, hit);
    }

    pub fn record_operation_identity(&self, identity: GraphQLSpanOperationIdentity) {
        if self.span.is_disabled() {
            return;
        }

        record_all!(
            self.span,
            "graphql.operation.name" = identity.name,
            "graphql.operation.type" = identity.operation_type,
            "graphql.document.hash" = identity.client_document_hash,
        );
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
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlValidate.into();
        let span = info_span!(
            target: TARGET_NAME,
            "graphql.validate",
            "hive.kind" = kind,
            "otel.kind" = "Internal",
            "cache.hit" = Empty,
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
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlVariableCoercion.into();
        let span = info_span!(
            target: TARGET_NAME,
            "graphql.variable_coercion",
            "hive.kind" = kind,
            "otel.kind" = "Internal",
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
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlNormalize.into();
        let span = info_span!(
            target: TARGET_NAME,
            "graphql.normalize",
            "hive.kind" = kind,
            "otel.kind" = "Internal",
            "cache.hit" = Empty,
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
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlAuthorize.into();
        let span = info_span!(
            target: TARGET_NAME,
            "graphql.authorize",
            "hive.kind" = kind,
            "otel.kind" = "Internal",
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
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlPlan.into();
        let span = info_span!(
            target: TARGET_NAME,
            "graphql.plan",
            "hive.kind" = kind,
            "otel.kind" = "Internal",
            "cache.hit" = Empty,
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
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlExecute.into();
        let span = info_span!(
            target: TARGET_NAME,
            "graphql.execute",
            "hive.kind" = kind,
            "otel.kind" = "Internal",
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
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphqlOperation.into();
        let span = info_span!(
            target: TARGET_NAME,
            "graphql.operation",
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
            "hive.graphql.operation.hash" = Empty,
            "hive.client.name" = Empty,
            "hive.client.version" = Empty,
        );
        GraphQLOperationSpan { span }
    }

    // pub fn record_document(&self, document: &str) {
    //     self.span
    //         .record(attributes::GRAPHQL_DOCUMENT_TEXT, document);
    // }

    // pub fn record_hive_operation_hash(&self, hash: &str) {
    //     self.span
    //         .record(attributes::HIVE_GRAPHQL_OPERATION_HASH, hash);
    // }

    pub fn record_error_count(&self, count: usize) {
        self.span
            .record(attributes::HIVE_GRAPHQL_ERROR_COUNT, count);
    }

    pub fn record_errors(&self, errors_fn: impl FnOnce() -> Vec<ObservedError>) {
        if self.is_disabled() {
            return;
        }

        let errors = errors_fn();
        record_error_codes_to_span(&self.span, &errors);
        record_error_events_to_span(&self.span, errors);
    }

    pub fn record_details(
        &self,
        document: &str,
        identity: GraphQLSpanOperationIdentity,
        client_name: Option<&str>,
        client_version: Option<&str>,
        hash: &str,
    ) {
        if self.span.is_disabled() {
            return;
        }

        record_all!(
            self.span,
            "graphql.document.text" = document,
            "graphql.operation.name" = identity.name,
            "graphql.operation.type" = identity.operation_type,
            "graphql.document.hash" = identity.client_document_hash,
            "hive.graphql.operation.hash" = hash,
            "hive.client.name" = client_name,
            "hive.client.version" = client_version,
        );
    }

    // pub fn record_operation_identity(&self, identity: GraphQLSpanOperationIdentity) {
    //     record_all!(
    //         self.span,
    //         "graphql.operation.name" = identity.name,
    //         "graphql.operation.type" = identity.operation_type,
    //         "graphql.document.hash" = identity.client_document_hash,
    //     );

    //     // if let Some(id) = &identity.document_id {
    //     //     self.span().record(attributes::GRAPHQL_OPERATION_ID, id.as_str());
    //     // }
    // }

    // pub fn record_client_identity(&self, client_name: Option<&str>, client_version: Option<&str>) {
    //     if self.span.is_disabled() {
    //         return;
    //     }

    //     record_all!(
    //         self.span,
    //         "hive.client.name" = client_name,
    //         "hive.client.version" = client_version,
    //     );
    // }
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
    pub fn new(subgraph_name: &str, document: &str) -> Self {
        if !is_level_enabled(Level::INFO) {
            return Self {
                span: disabled_span(),
            };
        }

        let kind: &'static str = HiveSpanKind::GraphQLSubgraphOperation.into();
        let span = info_span!(
            target: TARGET_NAME,
            "graphql.subgraph.operation",
            "hive.kind" = kind,
            "otel.status_code" = Empty,
            "otel.kind" = "Client",
            "error.type" = Empty,
            "graphql.operation.name" = Empty,
            "graphql.operation.type" = Empty,
            "graphql.document.hash" = Empty,
            "graphql.document.text" = document,
            "hive.graphql.error.count" = Empty,
            "hive.graphql.error.codes" = Empty,
            // Hive Console Attributes
            "hive.graphql.subgraph.name" = subgraph_name,
        );
        GraphQLSubgraphOperationSpan { span }
    }

    // pub fn record_document(&self, document: &str) {
    //     self.span
    //         .record(attributes::GRAPHQL_DOCUMENT_TEXT, document);
    // }

    pub fn record_error_count(&self, count: usize) {
        self.span
            .record(attributes::HIVE_GRAPHQL_ERROR_COUNT, count);
    }

    pub fn record_errors(&self, errors_fn: impl FnOnce() -> Vec<ObservedError>) {
        if self.is_disabled() {
            return;
        }

        let errors = errors_fn();
        record_error_codes_to_span(&self.span, &errors);
        record_error_events_to_span(&self.span, errors);
    }

    pub fn record_operation_identity(&self, identity: GraphQLSpanOperationIdentity) {
        record_all!(
            self.span,
            "graphql.operation.name" = identity.name,
            "graphql.operation.type" = identity.operation_type,
            "graphql.document.hash" = identity.client_document_hash,
        );
        // if let Some(name) = &identity.name {
        //     self.span.record(attributes::GRAPHQL_OPERATION_NAME, name);
        // }
        // self.span
        //     .record(attributes::GRAPHQL_OPERATION_TYPE, identity.operation_type);
        // self.span.record(
        //     attributes::GRAPHQL_DOCUMENT_HASH,
        //     identity.client_document_hash,
        // );
        // if let Some(id) = &identity.document_id {
        //     self.span().record(attributes::GRAPHQL_OPERATION_ID, id.as_str());
        // }
    }
}

fn record_error_codes_to_span(span: &Span, errors: &[ObservedError]) {
    let mut codes: Vec<&str> = errors.iter().filter_map(|e| e.code.as_deref()).collect();

    if codes.is_empty() {
        return;
    }

    codes.sort_unstable();
    codes.dedup();

    span.record(attributes::HIVE_GRAPHQL_ERROR_CODES, codes.join(","));
}

fn record_error_events_to_span(span: &Span, errors: Vec<ObservedError>) {
    if errors.is_empty() {
        return;
    }

    for error in errors {
        let message = &error.message;
        let mut attributes: Vec<KeyValue> = Vec::with_capacity(6);
        let kind: &'static str = HiveEventKind::GraphQLError.into();

        attributes.push(KeyValue::new(HIVE_KIND, kind));
        attributes.push(KeyValue::new(ERROR_MESSAGE, message.to_string()));
        attributes.push(KeyValue::new(
            ERROR_TYPE,
            error.code.unwrap_or(String::from("unknown")).to_string(),
        ));

        if let Some(service_name) = error.service_name {
            attributes.push(KeyValue::new(
                HIVE_ERROR_SUBGRAPH_NAME,
                service_name.to_string(),
            ));
        }

        if let Some(affected_path) = error.affected_path {
            attributes.push(KeyValue::new(
                HIVE_ERROR_AFFECTED_PATH,
                affected_path.to_string(),
            ));
        }

        if let Some(path) = error.path {
            attributes.push(KeyValue::new(HIVE_ERROR_PATH, path));
        }

        span.add_event(message.to_string(), attributes);
    }
}

pub struct GraphQLSpanOperationIdentity<'a> {
    pub name: Option<&'a str>,
    pub operation_type: &'a str,
    /// Hash of the original document sent to the router, by the client.
    pub client_document_hash: &'a str,
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::telemetry::traces::spans::attributes;

//     fn assert_fields(span: &Span, expected_fields: &[&str]) {
//         for field in expected_fields {
//             assert!(
//                 span.field(*field).is_some(),
//                 "Field '{}' is missing from span '{}'",
//                 field,
//                 span.metadata().expect("Span should have metadata").name()
//             );
//         }
//     }

//     #[test]
//     fn test_graphql_parse_span_fields() {
//         let span = GraphQLParseSpan::new();
//         assert_fields(
//             &span,
//             &[
//                 attributes::HIVE_KIND,
//                 attributes::OTEL_KIND,
//                 attributes::CACHE_HIT,
//                 attributes::GRAPHQL_OPERATION_NAME,
//                 attributes::GRAPHQL_OPERATION_TYPE,
//                 attributes::GRAPHQL_DOCUMENT_HASH,
//             ],
//         );
//     }

//     #[test]
//     fn test_graphql_validate_span_fields() {
//         let span = GraphQLValidateSpan::new();
//         assert_fields(
//             &span,
//             &[
//                 attributes::HIVE_KIND,
//                 attributes::OTEL_KIND,
//                 attributes::CACHE_HIT,
//             ],
//         );
//     }

//     #[test]
//     fn test_graphql_variable_coercion_span_fields() {
//         let span = GraphQLVariableCoercionSpan::new();
//         assert_fields(&span, &[attributes::HIVE_KIND, attributes::OTEL_KIND]);
//     }

//     #[test]
//     fn test_graphql_normalize_span_fields() {
//         let span = GraphQLNormalizeSpan::new();
//         assert_fields(
//             &span,
//             &[
//                 attributes::HIVE_KIND,
//                 attributes::OTEL_KIND,
//                 attributes::CACHE_HIT,
//             ],
//         );
//     }

//     #[test]
//     fn test_graphql_authorize_span_fields() {
//         let span = GraphQLAuthorizeSpan::new();
//         assert_fields(&span, &[attributes::HIVE_KIND, attributes::OTEL_KIND]);
//     }

//     #[test]
//     fn test_graphql_plan_span_fields() {
//         let span = GraphQLPlanSpan::new();
//         assert_fields(
//             &span,
//             &[
//                 attributes::HIVE_KIND,
//                 attributes::OTEL_KIND,
//                 attributes::CACHE_HIT,
//             ],
//         );
//     }

//     #[test]
//     fn test_graphql_execute_span_fields() {
//         let span = GraphQLExecuteSpan::new();
//         assert_fields(&span, &[attributes::HIVE_KIND, attributes::OTEL_KIND]);
//     }

//     #[test]
//     fn test_graphql_operation_span_fields() {
//         let span = GraphQLOperationSpan::new();
//         assert_fields(
//             &span,
//             &[
//                 attributes::HIVE_KIND,
//                 attributes::OTEL_STATUS_CODE,
//                 attributes::OTEL_KIND,
//                 attributes::ERROR_TYPE,
//                 attributes::GRAPHQL_OPERATION_NAME,
//                 attributes::GRAPHQL_OPERATION_TYPE,
//                 attributes::GRAPHQL_OPERATION_ID,
//                 attributes::GRAPHQL_DOCUMENT_HASH,
//                 attributes::GRAPHQL_DOCUMENT_TEXT,
//                 attributes::HIVE_GRAPHQL_ERROR_COUNT,
//                 attributes::HIVE_GRAPHQL_ERROR_CODES,
//                 attributes::HIVE_CLIENT_NAME,
//                 attributes::HIVE_CLIENT_VERSION,
//                 attributes::HIVE_GRAPHQL_OPERATION_HASH,
//             ],
//         );
//     }

//     #[test]
//     fn test_graphql_subgraph_operation_span_fields() {
//         let span = GraphQLSubgraphOperationSpan::new("test-subgraph");
//         assert_fields(
//             &span,
//             &[
//                 attributes::HIVE_KIND,
//                 attributes::OTEL_STATUS_CODE,
//                 attributes::OTEL_KIND,
//                 attributes::ERROR_TYPE,
//                 attributes::GRAPHQL_OPERATION_NAME,
//                 attributes::GRAPHQL_OPERATION_TYPE,
//                 attributes::GRAPHQL_DOCUMENT_HASH,
//                 attributes::GRAPHQL_DOCUMENT_TEXT,
//                 attributes::HIVE_GRAPHQL_ERROR_COUNT,
//                 attributes::HIVE_GRAPHQL_ERROR_CODES,
//                 attributes::HIVE_GRAPHQL_SUBGRAPH_NAME,
//             ],
//         );
//     }
// }
