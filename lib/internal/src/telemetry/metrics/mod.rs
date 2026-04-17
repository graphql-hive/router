pub mod cache_metrics;
mod capture;
pub mod catalog;
pub mod demand_control_metrics;
pub mod graphql_metrics;
pub mod http_client_metrics;
pub mod http_server_metrics;
pub mod persisted_documents_metrics;
pub mod setup;
pub mod supergraph_metrics;

pub use opentelemetry::metrics::ObservableGauge;
pub use setup::{build_meter_provider_from_config, MetricsSetup, PrometheusRuntimeConfig};

use opentelemetry::metrics::Meter;

use crate::telemetry::metrics::cache_metrics::CacheMetrics;
use crate::telemetry::metrics::demand_control_metrics::DemandControlMetrics;
use crate::telemetry::metrics::graphql_metrics::GraphQLMetrics;
use crate::telemetry::metrics::http_client_metrics::HttpClientMetrics;
use crate::telemetry::metrics::http_server_metrics::HttpServerMetrics;
use crate::telemetry::metrics::persisted_documents_metrics::PersistedDocumentsMetrics;
use crate::telemetry::metrics::supergraph_metrics::SupergraphMetrics;

pub struct Metrics {
    pub http_server: HttpServerMetrics,
    pub http_client: HttpClientMetrics,
    pub graphql: GraphQLMetrics,
    pub demand_control: DemandControlMetrics,
    pub supergraph: SupergraphMetrics,
    pub cache: CacheMetrics,
    pub persisted_documents: PersistedDocumentsMetrics,
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
            persisted_documents: PersistedDocumentsMetrics::new(meter),
        }
    }
}
