use ntex::web::HttpRequest;

use hive_router_internal::telemetry::metrics::catalog::values::GraphQLResponseStatus;

#[derive(Debug, Clone, Default)]
pub struct GraphQLOperationMetricIdentity {
    pub operation_name: Option<String>,
    pub operation_type: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct GraphQLResponseMetricStatus(pub GraphQLResponseStatus);

#[inline]
pub fn write_graphql_operation_metric_identity(
    req: &HttpRequest,
    operation_name: Option<String>,
    operation_type: Option<&'static str>,
) {
    req.extensions_mut().insert(GraphQLOperationMetricIdentity {
        operation_name,
        operation_type,
    });
}

#[inline]
pub fn read_graphql_operation_metric_identity(
    req: &HttpRequest,
) -> Option<GraphQLOperationMetricIdentity> {
    req.extensions()
        .get::<GraphQLOperationMetricIdentity>()
        .cloned()
}

#[inline]
pub fn write_graphql_response_metric_status(req: &HttpRequest, status: GraphQLResponseStatus) {
    req.extensions_mut()
        .insert(GraphQLResponseMetricStatus(status));
}

#[inline]
pub fn read_graphql_response_metric_status(req: &HttpRequest) -> Option<GraphQLResponseStatus> {
    req.extensions()
        .get::<GraphQLResponseMetricStatus>()
        .map(|status| status.0)
}
