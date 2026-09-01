pub mod cache_metrics;
mod capture;
pub mod catalog;
pub mod circuit_breaker_metrics;
pub mod coprocessor_metrics;
pub mod demand_control_metrics;
pub mod graphql_metrics;
pub mod http_client_metrics;
pub mod http_server_metrics;
pub mod persisted_documents_metrics;
pub mod request_dedupe_metrics;
pub mod setup;
pub mod subscription_metrics;
pub mod supergraph_metrics;
pub mod websocket_pool_metrics;
pub use setup::{build_meter_provider_from_config, PrometheusRuntimeConfig};

use opentelemetry::metrics::Meter;

use crate::telemetry::metrics::cache_metrics::CacheMetrics;
use crate::telemetry::metrics::circuit_breaker_metrics::CircuitBreakerMetrics;
use crate::telemetry::metrics::coprocessor_metrics::CoprocessorMetrics;
use crate::telemetry::metrics::demand_control_metrics::DemandControlMetrics;
use crate::telemetry::metrics::graphql_metrics::GraphQLMetrics;
use crate::telemetry::metrics::http_client_metrics::HttpClientMetrics;
use crate::telemetry::metrics::http_server_metrics::HttpServerMetrics;
use crate::telemetry::metrics::persisted_documents_metrics::PersistedDocumentsMetrics;
use crate::telemetry::metrics::request_dedupe_metrics::RequestDedupeMetrics;
use crate::telemetry::metrics::subscription_metrics::SubscriptionMetrics;
use crate::telemetry::metrics::supergraph_metrics::SupergraphMetrics;
use crate::telemetry::metrics::websocket_pool_metrics::WebSocketPoolMetrics;

pub struct Metrics {
    pub http_server: HttpServerMetrics,
    pub http_client: HttpClientMetrics,
    pub graphql: GraphQLMetrics,
    pub demand_control: DemandControlMetrics,
    pub supergraph: SupergraphMetrics,
    pub cache: CacheMetrics,
    pub circuit_breaker: CircuitBreakerMetrics,
    pub persisted_documents: PersistedDocumentsMetrics,
    pub coprocessor: CoprocessorMetrics,
    pub subscriptions: SubscriptionMetrics,
    pub websocket_pool: WebSocketPoolMetrics,
    pub request_dedupe: RequestDedupeMetrics,
}

impl Metrics {
    pub fn new(meter: Option<&Meter>) -> Self {
        Self {
            http_server: HttpServerMetrics::new(meter),
            http_client: HttpClientMetrics::new(meter),
            graphql: GraphQLMetrics::new(meter),
            demand_control: DemandControlMetrics::new(meter),
            supergraph: SupergraphMetrics::new(meter),
            cache: CacheMetrics::new(meter),
            circuit_breaker: CircuitBreakerMetrics::new(meter),
            persisted_documents: PersistedDocumentsMetrics::new(meter),
            coprocessor: CoprocessorMetrics::new(meter),
            subscriptions: SubscriptionMetrics::new(meter),
            websocket_pool: WebSocketPoolMetrics::new(meter),
            request_dedupe: RequestDedupeMetrics::new(meter),
        }
    }
}
