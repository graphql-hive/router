use parking_lot::RwLock;
use recloser::AsyncRecloser;
use reqwest::{
    header::{HeaderValue, IF_NONE_MATCH},
    StatusCode,
};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::RetryTransientMiddleware;

use crate::supergraph_fetcher::{
    builder::SupergraphFetcherBuilder, SupergraphFetcher, SupergraphFetcherError,
};

#[derive(Debug)]
pub struct SupergraphFetcherAsyncState {
    endpoints_with_circuit_breakers: Vec<(String, AsyncRecloser)>,
    reqwest_client: ClientWithMiddleware,
}

impl SupergraphFetcher<SupergraphFetcherAsyncState> {
    pub async fn fetch_supergraph(&self) -> Result<Option<String>, SupergraphFetcherError> {
        for (endpoint, circuit_breaker) in &self.state.endpoints_with_circuit_breakers {
            let Ok(resp) = self
                .try_fetch_from_endpoint(endpoint, circuit_breaker)
                .await
            else {
                continue;
            };

            // Got a successful response
            if matches!(resp.status(), StatusCode::NOT_MODIFIED) {
                return Ok(None);
            }

            self.update_latest_etag(resp.headers().get("etag"));
            let text = resp
                .text()
                .await
                .map_err(SupergraphFetcherError::ResponseParse)?;
            return Ok(Some(text));
        }

        Ok(None)
    }

    async fn try_fetch_from_endpoint(
        &self,
        endpoint: &str,
        circuit_breaker: &AsyncRecloser,
    ) -> Result<reqwest::Response, SupergraphFetcherError> {
        let mut req = self.state.reqwest_client.get(endpoint);

        if let Some(etag) = self.get_latest_etag() {
            req = req.header(IF_NONE_MATCH, etag);
        }

        let resp_fut = async {
            let resp = req.send().await.map_err(SupergraphFetcherError::Network)?;

            // Check for server errors (5xx)
            if resp.status().is_server_error() {
                return Err(SupergraphFetcherError::Network(
                    reqwest_middleware::Error::Middleware(anyhow::anyhow!(
                        "Server error: {}",
                        resp.status()
                    )),
                ));
            }

            Ok(resp)
        };

        circuit_breaker.call(resp_fut).await.map_err(|e| match e {
            recloser::Error::Inner(e) => e,
            recloser::Error::Rejected => SupergraphFetcherError::RejectedByCircuitBreaker,
        })
    }

    fn get_latest_etag(&self) -> Option<HeaderValue> {
        self.etag.read().clone()
    }

    fn update_latest_etag(&self, etag: Option<&HeaderValue>) {
        *self.etag.write() = etag.cloned();
    }
}

impl SupergraphFetcherBuilder {
    /// Builds an asynchronous SupergraphFetcher
    pub fn build_async(
        self,
    ) -> Result<SupergraphFetcher<SupergraphFetcherAsyncState>, SupergraphFetcherError> {
        self.validate_endpoints()?;

        let headers = self.prepare_headers()?;

        let mut reqwest_agent = reqwest::Client::builder()
            .danger_accept_invalid_certs(self.accept_invalid_certs)
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .default_headers(headers);

        if let Some(user_agent) = self.user_agent {
            reqwest_agent = reqwest_agent.user_agent(user_agent);
        }

        let reqwest_agent = reqwest_agent
            .build()
            .map_err(SupergraphFetcherError::HTTPClientCreation)?;
        let reqwest_client = ClientBuilder::new(reqwest_agent)
            .with(RetryTransientMiddleware::new_with_policy(self.retry_policy))
            .build();

        Ok(SupergraphFetcher {
            state: SupergraphFetcherAsyncState {
                reqwest_client,
                endpoints_with_circuit_breakers: self
                    .endpoints
                    .into_iter()
                    .map(|endpoint| {
                        let circuit_breaker = self
                            .circuit_breaker
                            .clone()
                            .unwrap_or_default()
                            .build_async()
                            .map_err(SupergraphFetcherError::CircuitBreakerCreation);
                        circuit_breaker.map(|cb| (endpoint, cb))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            etag: RwLock::new(None),
        })
    }
}
