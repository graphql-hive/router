use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use hive_router_internal::telemetry::logging::targets;
use serde::Deserialize;
use tracing::{debug, error, trace};

use crate::supergraph::base::{LoadSupergraphError, ReloadSupergraphResult, SupergraphLoader};

#[derive(Debug, thiserror::Error)]
pub enum ApolloGraphOSSupergraphError {
    #[error("Apollo GraphOS graph ref is missing. Please provide it via 'APOLLO_GRAPH_REF' environment variable or under 'supergraph.graph_ref' in the configuration.")]
    MissingApolloGraphRef,
    #[error("Apollo GraphOS key is missing. Please provide it via 'APOLLO_KEY' environment variable or under 'supergraph.key' in the configuration.")]
    MissingApolloKey,
    #[error("Failed to initialize the loader: {0}")]
    InitializationError(String),
    #[error("Apollo Uplink reported a fetch error: code={code} message={message}")]
    ApolloUplinkFetchError { code: String, message: String },
    #[error("Failed to fetch from Apollo Uplink: no endpoint succeeded")]
    ApolloUplinkAllEndpointsFailed,
}

const UPLINK_QUERY: &str = r#"
query($apiKey: String!, $graphRef: String!, $ifAfterId: ID) {
  routerConfig(ref: $graphRef, apiKey: $apiKey, ifAfterId: $ifAfterId) {
    __typename
    ... on RouterConfigResult {
      id
      supergraphSDL
      minDelaySeconds
    }
    ... on Unchanged {
      id
      minDelaySeconds
    }
    ... on FetchError {
      code
      message
    }
  }
}
"#;

const INITIAL_POLL_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct UplinkGraphQLResponse {
    data: Option<UplinkData>,
}

#[derive(Debug, Deserialize)]
struct UplinkData {
    #[serde(rename = "routerConfig")]
    router_config: RouterConfigResponse,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum RouterConfigResponse {
    RouterConfigResult {
        id: String,
        #[serde(rename = "supergraphSDL")]
        supergraph_sdl: String,
        #[serde(rename = "minDelaySeconds")]
        min_delay_seconds: f64,
    },
    Unchanged {
        id: String,
        #[serde(rename = "minDelaySeconds")]
        min_delay_seconds: f64,
    },
    FetchError {
        code: String,
        message: String,
    },
}

pub struct SupergraphApolloGraphOSLoader {
    client: reqwest::Client,
    endpoints: Vec<String>,
    graph_ref: String,
    key: String,
    last_id: RwLock<Option<String>>,
    current_interval: RwLock<Duration>,
}

impl SupergraphApolloGraphOSLoader {
    pub fn try_new(
        endpoints: Vec<String>,
        graph_ref: &str,
        key: &str,
        timeout: Duration,
        accept_invalid_certs: bool,
    ) -> Result<Box<Self>, ApolloGraphOSSupergraphError> {
        debug!(
          target: targets::SUPERGRAPH,
          endpoints = ?endpoints,
          graph_ref,
          "Creating supergraph source from Apollo GraphOS Uplink",
        );

        let client = reqwest::Client::builder()
            .timeout(timeout)
            .danger_accept_invalid_certs(accept_invalid_certs)
            .build()
            .map_err(|e| ApolloGraphOSSupergraphError::InitializationError(e.to_string()))?;

        Ok(Box::new(Self {
            client,
            endpoints,
            graph_ref: graph_ref.to_string(),
            key: key.to_string(),
            last_id: RwLock::new(None),
            current_interval: RwLock::new(INITIAL_POLL_INTERVAL),
        }))
    }

    // Tries each configured endpoint in order, a failed/unparseable response just
    // falls through to the next endpoint, and the next scheduled poll is the real retry mechanism.
    async fn fetch(&self) -> Result<RouterConfigResponse, ApolloGraphOSSupergraphError> {
        let last_id = self.last_id.read().unwrap().clone();

        let body = serde_json::json!({
            "query": UPLINK_QUERY,
            "variables": {
                "apiKey": &self.key,
                "graphRef": &self.graph_ref,
                "ifAfterId": last_id,
            },
        });

        for endpoint in &self.endpoints {
            let response = match self.client.post(endpoint).json(&body).send().await {
                Ok(response) => response,
                Err(err) => {
                    debug!(
                      target: targets::SUPERGRAPH,
                      endpoint,
                      error = ?err,
                      "failed to fetch from Apollo Uplink endpoint, trying next endpoint",
                    );

                    continue;
                }
            };

            match response.json::<UplinkGraphQLResponse>().await {
                Ok(UplinkGraphQLResponse { data: Some(data) }) => {
                    return Ok(data.router_config);
                }
                Ok(UplinkGraphQLResponse { data: None }) => {
                    debug!(
                      target: targets::SUPERGRAPH,
                      endpoint,
                      "empty response from Apollo Uplink endpoint, trying next endpoint",
                    );
                }
                Err(err) => {
                    debug!(
                      target: targets::SUPERGRAPH,
                      endpoint,
                      error = ?err,
                      "failed to parse response from Apollo Uplink endpoint, trying next endpoint",
                    );
                }
            }
        }

        Err(ApolloGraphOSSupergraphError::ApolloUplinkAllEndpointsFailed)
    }
}

#[async_trait]
impl SupergraphLoader for SupergraphApolloGraphOSLoader {
    async fn load(&self) -> Result<ReloadSupergraphResult, LoadSupergraphError> {
        let router_config = self.fetch().await?;

        match router_config {
            RouterConfigResponse::RouterConfigResult {
                id,
                supergraph_sdl,
                min_delay_seconds,
            } => {
                *self.last_id.write().unwrap() = Some(id);
                *self.current_interval.write().unwrap() =
                    Duration::from_secs_f64(min_delay_seconds.max(0.0));

                trace!(
                  target: targets::SUPERGRAPH,
                  "supergraph loaded from Apollo Uplink with changes",
                );

                Ok(ReloadSupergraphResult::Changed {
                    new_sdl: supergraph_sdl,
                })
            }
            RouterConfigResponse::Unchanged {
                id,
                min_delay_seconds,
            } => {
                *self.last_id.write().unwrap() = Some(id);
                *self.current_interval.write().unwrap() =
                    Duration::from_secs_f64(min_delay_seconds.max(0.0));

                Ok(ReloadSupergraphResult::Unchanged)
            }
            RouterConfigResponse::FetchError { code, message } => {
                error!(
                  target: targets::SUPERGRAPH,
                  code = %code,
                  message = %message,
                  "Apollo Uplink reported a fetch error",
                );

                Err(ApolloGraphOSSupergraphError::ApolloUplinkFetchError { code, message }.into())
            }
        }
    }

    fn reload_interval(&self) -> Option<Duration> {
        Some(*self.current_interval.read().unwrap())
    }
}
