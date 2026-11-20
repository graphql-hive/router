use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use ntex::web::HttpRequest;

use crate::telemetry::metrics::catalog::values::GraphQLResponseStatus;

#[derive(Debug, Clone, Default)]
pub struct GraphQLOperationMetricIdentity {
    pub operation_name: Option<String>,
    pub operation_type: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct GraphQLResponseMetricStatus(pub GraphQLResponseStatus);

/// The error count of an incremental (`@defer`) response, every part's once the last one is
/// out. The HTTP metrics of such a response are recorded then, not with its head.
#[derive(Debug, Clone)]
pub struct DeferredResponseErrorCount(pub Arc<AtomicUsize>);

#[inline]
pub fn write_deferred_response_error_count(req: &HttpRequest, error_count: Arc<AtomicUsize>) {
    req.extensions_mut()
        .insert(DeferredResponseErrorCount(error_count));
}

#[inline]
pub fn read_deferred_response_error_count(req: &HttpRequest) -> Option<Arc<AtomicUsize>> {
    req.extensions()
        .get::<DeferredResponseErrorCount>()
        .map(|count| count.0.clone())
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
