#[cfg(debug_assertions)]
use opentelemetry::KeyValue;

pub mod values {
    pub const UNKNOWN: &str = "UNKNOWN";

    #[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
    pub enum SupergraphPollResult {
        #[strum(serialize = "updated")]
        Updated,
        #[strum(serialize = "not_modified")]
        NotModified,
        #[strum(serialize = "error")]
        Error,
    }

    impl SupergraphPollResult {
        pub fn as_str(self) -> &'static str {
            self.into()
        }
    }

    #[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
    pub enum SupergraphProcessStatus {
        #[strum(serialize = "ok")]
        Ok,
        #[strum(serialize = "error")]
        Error,
    }

    impl SupergraphProcessStatus {
        pub fn as_str(self) -> &'static str {
            self.into()
        }
    }

    #[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
    pub enum GraphQLResponseStatus {
        #[strum(serialize = "ok")]
        Ok,
        #[strum(serialize = "error")]
        Error,
    }

    impl GraphQLResponseStatus {
        pub fn as_str(self) -> &'static str {
            self.into()
        }
    }

    #[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
    pub enum CacheResult {
        #[strum(serialize = "hit")]
        Hit,
        #[strum(serialize = "miss")]
        Miss,
    }

    impl CacheResult {
        pub fn as_str(self) -> &'static str {
            self.into()
        }
    }
}

pub mod labels {
    pub const CODE: &str = "code";
    pub const RESULT: &str = "result";
    pub const STATUS: &str = "status";
    pub const ERROR_TYPE: &str = "error.type";
    pub const SUBGRAPH_NAME: &str = "subgraph.name";
    pub const HTTP_REQUEST_METHOD: &str = "http.request.method";
    pub const HTTP_RESPONSE_STATUS_CODE: &str = "http.response.status_code";
    pub const HTTP_ROUTE: &str = "http.route";
    pub const URL_SCHEME: &str = "url.scheme";
    pub const NETWORK_PROTOCOL_NAME: &str = "network.protocol.name";
    pub const NETWORK_PROTOCOL_VERSION: &str = "network.protocol.version";
    pub const SERVER_ADDRESS: &str = "server.address";
    pub const SERVER_PORT: &str = "server.port";
    pub const GRAPHQL_OPERATION_TYPE: &str = "graphql.operation.type";
    pub const GRAPHQL_OPERATION_NAME: &str = "graphql.operation.name";
    pub const GRAPHQL_RESPONSE_STATUS: &str = "graphql.response.status";
}

pub mod names {
    pub const GRAPHQL_ERRORS_TOTAL: &str = "hive.router.graphql.errors_total";
    pub const SUPERGRAPH_POLL_TOTAL: &str = "hive.router.supergraph.poll.total";
    pub const SUPERGRAPH_POLL_DURATION: &str = "hive.router.supergraph.poll.duration";
    pub const SUPERGRAPH_PROCESS_DURATION: &str = "hive.router.supergraph.process.duration";
    pub const HTTP_SERVER_REQUEST_DURATION: &str = "http.server.request.duration";
    pub const HTTP_SERVER_ACTIVE_REQUESTS: &str = "http.server.active_requests";
    pub const HTTP_SERVER_REQUEST_BODY_SIZE: &str = "http.server.request.body.size";
    pub const HTTP_SERVER_RESPONSE_BODY_SIZE: &str = "http.server.response.body.size";
    pub const HTTP_CLIENT_REQUEST_DURATION: &str = "http.client.request.duration";
    pub const HTTP_CLIENT_ACTIVE_REQUESTS: &str = "http.client.active_requests";
    pub const HTTP_CLIENT_REQUEST_BODY_SIZE: &str = "http.client.request.body.size";
    pub const HTTP_CLIENT_RESPONSE_BODY_SIZE: &str = "http.client.response.body.size";
    pub const PARSE_CACHE_REQUESTS_TOTAL: &str = "hive.router.parse_cache.requests_total";
    pub const PARSE_CACHE_DURATION: &str = "hive.router.parse_cache.duration";
    pub const PARSE_CACHE_SIZE: &str = "hive.router.parse_cache.size";
    pub const VALIDATE_CACHE_REQUESTS_TOTAL: &str = "hive.router.validate_cache.requests_total";
    pub const VALIDATE_CACHE_DURATION: &str = "hive.router.validate_cache.duration";
    pub const VALIDATE_CACHE_SIZE: &str = "hive.router.validate_cache.size";
    pub const NORMALIZE_CACHE_REQUESTS_TOTAL: &str = "hive.router.normalize_cache.requests_total";
    pub const NORMALIZE_CACHE_DURATION: &str = "hive.router.normalize_cache.duration";
    pub const NORMALIZE_CACHE_SIZE: &str = "hive.router.normalize_cache.size";
    pub const PLAN_CACHE_REQUESTS_TOTAL: &str = "hive.router.plan_cache.requests_total";
    pub const PLAN_CACHE_DURATION: &str = "hive.router.plan_cache.duration";
    pub const PLAN_CACHE_SIZE: &str = "hive.router.plan_cache.size";
}

pub(crate) const METRIC_SPECS: &[(&str, &[&str])] = &[
    (names::GRAPHQL_ERRORS_TOTAL, &[labels::CODE]),
    (
        names::HTTP_SERVER_REQUEST_DURATION,
        &[
            labels::HTTP_REQUEST_METHOD,
            labels::HTTP_RESPONSE_STATUS_CODE,
            labels::HTTP_ROUTE,
            labels::NETWORK_PROTOCOL_NAME,
            labels::NETWORK_PROTOCOL_VERSION,
            labels::URL_SCHEME,
            labels::ERROR_TYPE,
            labels::GRAPHQL_OPERATION_NAME,
            labels::GRAPHQL_OPERATION_TYPE,
            labels::GRAPHQL_RESPONSE_STATUS,
        ],
    ),
    (
        names::HTTP_SERVER_REQUEST_BODY_SIZE,
        &[
            labels::HTTP_REQUEST_METHOD,
            labels::HTTP_RESPONSE_STATUS_CODE,
            labels::HTTP_ROUTE,
            labels::NETWORK_PROTOCOL_NAME,
            labels::NETWORK_PROTOCOL_VERSION,
            labels::URL_SCHEME,
            labels::ERROR_TYPE,
            labels::GRAPHQL_OPERATION_NAME,
            labels::GRAPHQL_OPERATION_TYPE,
            labels::GRAPHQL_RESPONSE_STATUS,
        ],
    ),
    (
        names::HTTP_SERVER_RESPONSE_BODY_SIZE,
        &[
            labels::HTTP_REQUEST_METHOD,
            labels::HTTP_RESPONSE_STATUS_CODE,
            labels::HTTP_ROUTE,
            labels::NETWORK_PROTOCOL_NAME,
            labels::NETWORK_PROTOCOL_VERSION,
            labels::URL_SCHEME,
            labels::ERROR_TYPE,
            labels::GRAPHQL_OPERATION_NAME,
            labels::GRAPHQL_OPERATION_TYPE,
            labels::GRAPHQL_RESPONSE_STATUS,
        ],
    ),
    (
        names::HTTP_SERVER_ACTIVE_REQUESTS,
        &[
            labels::HTTP_REQUEST_METHOD,
            labels::NETWORK_PROTOCOL_NAME,
            labels::URL_SCHEME,
        ],
    ),
    (
        names::HTTP_CLIENT_REQUEST_DURATION,
        &[
            labels::HTTP_REQUEST_METHOD,
            labels::SERVER_ADDRESS,
            labels::SERVER_PORT,
            labels::NETWORK_PROTOCOL_NAME,
            labels::NETWORK_PROTOCOL_VERSION,
            labels::URL_SCHEME,
            labels::SUBGRAPH_NAME,
            labels::HTTP_RESPONSE_STATUS_CODE,
            labels::ERROR_TYPE,
            labels::GRAPHQL_RESPONSE_STATUS,
        ],
    ),
    (
        names::HTTP_CLIENT_REQUEST_BODY_SIZE,
        &[
            labels::HTTP_REQUEST_METHOD,
            labels::SERVER_ADDRESS,
            labels::SERVER_PORT,
            labels::NETWORK_PROTOCOL_NAME,
            labels::NETWORK_PROTOCOL_VERSION,
            labels::URL_SCHEME,
            labels::SUBGRAPH_NAME,
            labels::HTTP_RESPONSE_STATUS_CODE,
            labels::ERROR_TYPE,
            labels::GRAPHQL_RESPONSE_STATUS,
        ],
    ),
    (
        names::HTTP_CLIENT_RESPONSE_BODY_SIZE,
        &[
            labels::HTTP_REQUEST_METHOD,
            labels::SERVER_ADDRESS,
            labels::SERVER_PORT,
            labels::NETWORK_PROTOCOL_NAME,
            labels::NETWORK_PROTOCOL_VERSION,
            labels::URL_SCHEME,
            labels::SUBGRAPH_NAME,
            labels::HTTP_RESPONSE_STATUS_CODE,
            labels::ERROR_TYPE,
            labels::GRAPHQL_RESPONSE_STATUS,
        ],
    ),
    (
        names::HTTP_CLIENT_ACTIVE_REQUESTS,
        &[
            labels::HTTP_REQUEST_METHOD,
            labels::SERVER_ADDRESS,
            labels::SERVER_PORT,
            labels::URL_SCHEME,
            labels::SUBGRAPH_NAME,
        ],
    ),
    (names::SUPERGRAPH_POLL_TOTAL, &[labels::RESULT]),
    (names::SUPERGRAPH_POLL_DURATION, &[labels::RESULT]),
    (names::SUPERGRAPH_PROCESS_DURATION, &[labels::STATUS]),
    (names::PARSE_CACHE_REQUESTS_TOTAL, &[labels::RESULT]),
    (names::PARSE_CACHE_DURATION, &[labels::RESULT]),
    (names::PARSE_CACHE_SIZE, &[]),
    (names::VALIDATE_CACHE_REQUESTS_TOTAL, &[labels::RESULT]),
    (names::VALIDATE_CACHE_DURATION, &[labels::RESULT]),
    (names::VALIDATE_CACHE_SIZE, &[]),
    (names::NORMALIZE_CACHE_REQUESTS_TOTAL, &[labels::RESULT]),
    (names::NORMALIZE_CACHE_DURATION, &[labels::RESULT]),
    (names::NORMALIZE_CACHE_SIZE, &[]),
    (names::PLAN_CACHE_REQUESTS_TOTAL, &[labels::RESULT]),
    (names::PLAN_CACHE_DURATION, &[labels::RESULT]),
    (names::PLAN_CACHE_SIZE, &[]),
];

pub fn labels_for(metric_name: &str) -> Option<&'static [&'static str]> {
    METRIC_SPECS
        .iter()
        .find(|(name, _)| *name == metric_name)
        .map(|(_, labels)| *labels)
}

pub(crate) fn all_metric_names() -> Vec<&'static str> {
    METRIC_SPECS.iter().map(|(name, _)| *name).collect()
}

#[cfg(debug_assertions)]
pub(crate) fn debug_assert_attrs(metric_name: &'static str, attrs: &[KeyValue]) {
    let labels = labels_for(metric_name)
        .unwrap_or_else(|| panic!("missing metric catalog entry for {metric_name}"));

    for attr in attrs {
        debug_assert!(
            labels.contains(&attr.key.as_str()),
            "attribute '{}' is not declared for metric '{}'",
            attr.key.as_str(),
            metric_name
        );
    }
}
