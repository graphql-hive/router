use ntex::web::HttpRequest;

use hive_router_internal::telemetry::metrics::catalog::values::GraphQLResponseStatus;

/// Stores the request body size in bytes.
///
/// The value comes from either:
/// - the `Content-Length` header
/// - the streamed payload, measured from bytes read.
///
/// For streamed payloads, the recorded size is the number of bytes read up to
/// the configured maximum.
///
/// Using `RequestBodySize` to store the size of the request body,
/// helps to reduce complexity in code, as otherwise,
/// we would have to return the size next within Err and Ok of `read_body_stream`.
#[derive(Debug, Clone, Copy)]
pub struct RequestBodySize(pub u64);

#[derive(Debug, Clone, Default)]
pub struct GraphQLOperationMetricIdentity {
    pub operation_name: Option<String>,
    pub operation_type: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct GraphQLResponseMetricStatus(pub GraphQLResponseStatus);

#[inline]
pub fn write_request_body_size(req: &HttpRequest, size: u64) {
    req.extensions_mut().insert(RequestBodySize(size));
}

#[inline]
pub fn read_request_body_size(req: &HttpRequest) -> Option<u64> {
    req.extensions().get::<RequestBodySize>().map(|size| size.0)
}

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
