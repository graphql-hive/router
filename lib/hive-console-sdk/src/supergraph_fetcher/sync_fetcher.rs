use std::time::SystemTime;

use parking_lot::RwLock;
use recloser::Recloser;
use reqwest::{
    header::{HeaderValue, IF_NONE_MATCH},
    StatusCode,
};
use reqwest_retry::{RetryDecision, RetryPolicy};
use retry_policies::policies::ExponentialBackoff;

use crate::supergraph_fetcher::{
    builder::SupergraphFetcherBuilder, SupergraphFetcher, SupergraphFetcherError,
};

#[derive(Debug)]
pub struct SupergraphFetcherSyncState {
    endpoints_with_circuit_breakers: Vec<(String, Recloser)>,
    reqwest_client: reqwest::blocking::Client,
    retry_policy: ExponentialBackoff,
}

impl SupergraphFetcher<SupergraphFetcherSyncState> {
    pub fn fetch_supergraph(&self) -> Result<Option<String>, SupergraphFetcherError> {
        for (endpoint, circuit_breaker) in &self.state.endpoints_with_circuit_breakers {
            let Ok(resp) = self.try_fetch_from_endpoint(endpoint, circuit_breaker) else {
                continue;
            };

            // Got a successful response
            if matches!(resp.status(), StatusCode::NOT_MODIFIED) {
                return Ok(None);
            }

            self.update_latest_etag(resp.headers().get("etag"));
            let text = resp.text().map_err(SupergraphFetcherError::ResponseParse)?;
            return Ok(Some(text));
        }

        Ok(None)
    }

    fn try_fetch_from_endpoint(
        &self,
        endpoint: &str,
        circuit_breaker: &Recloser,
    ) -> Result<reqwest::blocking::Response, SupergraphFetcherError> {
        circuit_breaker
            .call(|| self.send_with_retries(endpoint))
            .map_err(|e| match e {
                recloser::Error::Inner(e) => e,
                recloser::Error::Rejected => SupergraphFetcherError::RejectedByCircuitBreaker,
            })
    }

    fn send_with_retries(
        &self,
        endpoint: &str,
    ) -> Result<reqwest::blocking::Response, SupergraphFetcherError> {
        let request_start_time = SystemTime::now();
        let mut n_past_retries = 0;

        loop {
            let mut req = self.state.reqwest_client.get(endpoint);
            if let Some(etag) = self.get_latest_etag() {
                req = req.header(IF_NONE_MATCH, etag);
            }

            let response = req.send().map_err(|err| {
                SupergraphFetcherError::Network(reqwest_middleware::Error::Reqwest(err))
            });

            // Check for server errors (5xx)
            let response = response.and_then(|resp| {
                if resp.status().is_server_error() {
                    return Err(SupergraphFetcherError::Network(
                        reqwest_middleware::Error::Middleware(anyhow::anyhow!(
                            "Server error: {}",
                            resp.status()
                        )),
                    ));
                }
                Ok(resp)
            });

            let Ok(resp) = response else {
                let Err(e) = response else { unreachable!() };

                // Determine retry
                let RetryDecision::Retry { execute_after } = self
                    .state
                    .retry_policy
                    .should_retry(request_start_time, n_past_retries)
                else {
                    return Err(e);
                };

                n_past_retries += 1;
                let duration = execute_after.elapsed().map_err(|err| {
                    tracing::error!("Error determining sleep duration for retry: {}", err);
                    e
                })?;
                std::thread::sleep(duration);
                continue;
            };

            return Ok(resp);
        }
    }

    fn get_latest_etag(&self) -> Option<HeaderValue> {
        self.etag.read().clone()
    }

    fn update_latest_etag(&self, etag: Option<&HeaderValue>) {
        *self.etag.write() = etag.cloned();
    }
}

impl SupergraphFetcherBuilder {
    /// Builds a synchronous SupergraphFetcher
    pub fn build_sync(
        self,
    ) -> Result<SupergraphFetcher<SupergraphFetcherSyncState>, SupergraphFetcherError> {
        self.validate_endpoints()?;
        let headers = self.prepare_headers()?;

        let mut reqwest_client = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(self.accept_invalid_certs)
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .default_headers(headers);

        if let Some(user_agent) = &self.user_agent {
            reqwest_client = reqwest_client.user_agent(user_agent);
        }

        let reqwest_client = reqwest_client
            .build()
            .map_err(SupergraphFetcherError::HTTPClientCreation)?;
        let fetcher = SupergraphFetcher {
            state: SupergraphFetcherSyncState {
                reqwest_client,
                retry_policy: self.retry_policy,
                endpoints_with_circuit_breakers: self
                    .endpoints
                    .into_iter()
                    .map(|endpoint| {
                        let circuit_breaker = self
                            .circuit_breaker
                            .clone()
                            .unwrap_or_default()
                            .build_sync()
                            .map_err(SupergraphFetcherError::CircuitBreakerCreation);
                        circuit_breaker.map(|cb| (endpoint, cb))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            etag: RwLock::new(None),
        };
        Ok(fetcher)
    }
}
