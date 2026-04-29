use std::time::Duration;

use crate::agent::usage_agent::non_empty_string;
use crate::circuit_breaker::CircuitBreakerBuilder;
use crate::circuit_breaker::CircuitBreakerError;
use moka::future::Cache;
use recloser::AsyncRecloser;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use reqwest_middleware::ClientBuilder;
use reqwest_middleware::ClientWithMiddleware;
use reqwest_retry::RetryTransientMiddleware;
use retry_policies::policies::ExponentialBackoff;
use tracing::{debug, info, warn};

#[derive(Debug)]
pub struct PersistedDocumentsManager {
    client: ClientWithMiddleware,
    cache: Cache<String, String>,
    negative_cache: Option<Cache<String, ()>>,
    endpoints_with_circuit_breakers: Vec<(String, AsyncRecloser)>,
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum PersistedDocumentsError {
    #[error("Failed to read body: {0}")]
    FailedToReadBody(String),
    #[error("Failed to parse body: {0}")]
    FailedToParseBody(String),
    #[error("Persisted document not found.")]
    DocumentNotFound,
    #[error("Failed to locate the persisted document key in request.")]
    KeyNotFound,
    #[error("Failed to validate persisted document")]
    FailedToFetchFromCDN(String),
    #[error("Failed to read CDN response body")]
    FailedToReadCDNResponse(String),
    #[error("No persisted document provided, or document id cannot be resolved.")]
    PersistedDocumentRequired,
    #[error("Missing required configuration option: {0}")]
    MissingConfigurationOption(String),
    #[error("Invalid CDN key {0}")]
    InvalidCDNKey(String),
    #[error("Failed to create HTTP client: {0}")]
    HTTPClientCreationError(String),
    #[error("unable to create circuit breaker: {0}")]
    CircuitBreakerCreationError(String),
    #[error("rejected by the circuit breaker")]
    CircuitBreakerRejected,
    #[error("unknown error")]
    Unknown,
}

impl From<reqwest_middleware::Error> for PersistedDocumentsError {
    fn from(err: reqwest_middleware::Error) -> Self {
        PersistedDocumentsError::FailedToFetchFromCDN(err.to_string())
    }
}

impl From<serde_json::Error> for PersistedDocumentsError {
    fn from(err: serde_json::Error) -> Self {
        PersistedDocumentsError::FailedToParseBody(err.to_string())
    }
}

impl From<CircuitBreakerError> for PersistedDocumentsError {
    fn from(err: CircuitBreakerError) -> Self {
        PersistedDocumentsError::CircuitBreakerCreationError(err.to_string())
    }
}

impl PersistedDocumentsError {
    pub fn message(&self) -> String {
        self.to_string()
    }

    pub fn code(&self) -> String {
        match self {
            PersistedDocumentsError::FailedToReadBody(_) => "FAILED_TO_READ_BODY".into(),
            PersistedDocumentsError::FailedToParseBody(_) => "FAILED_TO_PARSE_BODY".into(),
            PersistedDocumentsError::DocumentNotFound => "PERSISTED_DOCUMENT_NOT_FOUND".into(),
            PersistedDocumentsError::KeyNotFound => "PERSISTED_DOCUMENT_KEY_NOT_FOUND".into(),
            PersistedDocumentsError::FailedToFetchFromCDN(_) => "FAILED_TO_FETCH_FROM_CDN".into(),
            PersistedDocumentsError::FailedToReadCDNResponse(_) => {
                "FAILED_TO_READ_CDN_RESPONSE".into()
            }
            PersistedDocumentsError::PersistedDocumentRequired => {
                "PERSISTED_DOCUMENT_REQUIRED".into()
            }
            PersistedDocumentsError::MissingConfigurationOption(_) => {
                "MISSING_CONFIGURATION_OPTION".into()
            }
            PersistedDocumentsError::InvalidCDNKey(_) => "INVALID_CDN_KEY".into(),
            PersistedDocumentsError::HTTPClientCreationError(_) => {
                "HTTP_CLIENT_CREATION_ERROR".into()
            }
            PersistedDocumentsError::CircuitBreakerCreationError(_) => {
                "CIRCUIT_BREAKER_CREATION_ERROR".into()
            }
            PersistedDocumentsError::CircuitBreakerRejected => "CIRCUIT_BREAKER_REJECTED".into(),
            PersistedDocumentsError::Unknown => "UNKNOWN_ERROR".into(),
        }
    }
}

impl PersistedDocumentsManager {
    pub fn builder() -> PersistedDocumentsManagerBuilder {
        PersistedDocumentsManagerBuilder::default()
    }
    async fn resolve_from_endpoint(
        &self,
        endpoint: &str,
        document_id: &str,
        circuit_breaker: &AsyncRecloser,
    ) -> Result<String, PersistedDocumentsError> {
        let cdn_document_id = str::replace(document_id, "~", "/");
        let cdn_artifact_url = format!("{}/apps/{}", endpoint, cdn_document_id);
        info!(document_id, cdn_artifact_url, "fetching document from CDN");
        let response_fut = self.client.get(cdn_artifact_url).send();

        let response = circuit_breaker
            .call(response_fut)
            .await
            .map_err(|e| match e {
                recloser::Error::Inner(e) => PersistedDocumentsError::from(e),
                recloser::Error::Rejected => PersistedDocumentsError::CircuitBreakerRejected,
            })?;

        if response.status().is_success() {
            let document = response
                .text()
                .await
                .map_err(|e| PersistedDocumentsError::FailedToReadCDNResponse(e.to_string()))?;
            debug!(document_id, document, "Document fetched from CDN");

            return Ok(document);
        }

        let status = response.status();

        warn!(
            status = ?status,
            response = response
                .text()
                .await
                .unwrap_or_else(|_| "Unavailable".to_string()),
            "Document fetch from CDN failed"
        );

        Err(PersistedDocumentsError::DocumentNotFound)
    }

    /// Resolves the document from the cache, or from the CDN
    pub async fn resolve_document(
        &self,
        document_id: &str,
    ) -> Result<String, PersistedDocumentsError> {
        if let Some(negative_cache) = &self.negative_cache {
            if negative_cache.get(document_id).await.is_some() {
                debug!(
                    "Document {} found in negative cache, skipping CDN fetch",
                    document_id
                );
                return Err(PersistedDocumentsError::DocumentNotFound);
            }
        }

        if let Some(cached_document) = self.cache.get(document_id).await {
            return Ok(cached_document);
        }

        let result = self
            .cache
            .try_get_with_by_ref(document_id, async {
                debug!(
                    document_id,
                    "Document not found in cache. Fetching from CDN",
                );

                let mut last_error: Option<PersistedDocumentsError> = None;
                for (endpoint, circuit_breaker) in self.endpoints_with_circuit_breakers.iter() {
                    match self
                        .resolve_from_endpoint(endpoint, document_id, circuit_breaker)
                        .await
                    {
                        Ok(document) => return Ok(document),
                        Err(error) => last_error = Some(error),
                    }
                }

                Err(last_error.unwrap_or(PersistedDocumentsError::Unknown))
            })
            .await
            .map_err(|error| error.as_ref().clone());

        if matches!(&result, Err(PersistedDocumentsError::DocumentNotFound)) {
            if let Some(negative_cache) = &self.negative_cache {
                negative_cache.insert(document_id.to_string(), ()).await;
            }
        }

        result
    }
}

pub struct PersistedDocumentsManagerBuilder {
    key: Option<String>,
    endpoints: Vec<String>,
    accept_invalid_certs: bool,
    connect_timeout: Duration,
    request_timeout: Duration,
    retry_policy: ExponentialBackoff,
    cache_size: u64,
    negative_cache_ttl: Option<Duration>,
    user_agent: Option<String>,
    circuit_breaker: CircuitBreakerBuilder,
}

impl Default for PersistedDocumentsManagerBuilder {
    fn default() -> Self {
        Self {
            key: None,
            endpoints: vec![],
            accept_invalid_certs: false,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(15),
            retry_policy: ExponentialBackoff::builder().build_with_max_retries(3),
            cache_size: 10_000,
            negative_cache_ttl: None,
            user_agent: None,
            circuit_breaker: CircuitBreakerBuilder::default(),
        }
    }
}

impl PersistedDocumentsManagerBuilder {
    /// The CDN Access Token with from the Hive Console target.
    pub fn key(mut self, key: String) -> Self {
        self.key = non_empty_string(Some(key));
        self
    }

    /// The CDN endpoint from Hive Console target.
    pub fn add_endpoint(mut self, endpoint: String) -> Self {
        if let Some(endpoint) = non_empty_string(Some(endpoint)) {
            self.endpoints.push(endpoint);
        }
        self
    }

    /// Accept invalid SSL certificates
    /// default: false
    pub fn accept_invalid_certs(mut self, accept_invalid_certs: bool) -> Self {
        self.accept_invalid_certs = accept_invalid_certs;
        self
    }

    /// Connection timeout for the Hive Console CDN requests.
    /// Default: 5 seconds
    pub fn connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }

    /// Request timeout for the Hive Console CDN requests.
    /// Default: 15 seconds
    pub fn request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    /// Retry policy for fetching persisted documents
    /// Default: ExponentialBackoff with max 3 retries
    pub fn retry_policy(mut self, retry_policy: ExponentialBackoff) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    /// Maximum number of retries for fetching persisted documents
    /// Default: ExponentialBackoff with max 3 retries
    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.retry_policy = ExponentialBackoff::builder().build_with_max_retries(max_retries);
        self
    }

    /// Size of the in-memory cache for persisted documents
    /// Default: 10,000 entries
    pub fn cache_size(mut self, cache_size: u64) -> Self {
        self.cache_size = cache_size;
        self
    }

    /// TTL for negative cache entries (failed lookups / not found responses).
    ///
    /// When set, repeated misses for the same document id are served from in-memory cache
    /// until the TTL expires.
    pub fn negative_cache_ttl(mut self, ttl: Duration) -> Self {
        self.negative_cache_ttl = Some(ttl);
        self
    }

    /// Circuit breaker configuration for persisted document CDN requests.
    pub fn circuit_breaker(mut self, circuit_breaker: CircuitBreakerBuilder) -> Self {
        self.circuit_breaker = circuit_breaker;
        self
    }

    /// User-Agent header to be sent with each request
    pub fn user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = non_empty_string(Some(user_agent));
        self
    }

    pub fn build(self) -> Result<PersistedDocumentsManager, PersistedDocumentsError> {
        let mut default_headers = HeaderMap::new();
        let key = match self.key {
            Some(key) => key,
            None => {
                return Err(PersistedDocumentsError::MissingConfigurationOption(
                    "key".to_string(),
                ));
            }
        };
        default_headers.insert(
            "X-Hive-CDN-Key",
            HeaderValue::from_str(&key)
                .map_err(|e| PersistedDocumentsError::InvalidCDNKey(e.to_string()))?,
        );
        let mut reqwest_agent = reqwest::Client::builder()
            .danger_accept_invalid_certs(self.accept_invalid_certs)
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .default_headers(default_headers);

        if let Some(user_agent) = self.user_agent {
            reqwest_agent = reqwest_agent.user_agent(user_agent);
        }

        let reqwest_agent = reqwest_agent
            .build()
            .map_err(|e| PersistedDocumentsError::HTTPClientCreationError(e.to_string()))?;
        let client = ClientBuilder::new(reqwest_agent)
            .with(RetryTransientMiddleware::new_with_policy(self.retry_policy))
            .build();

        let cache = Cache::<String, String>::new(self.cache_size);
        let negative_cache = self.negative_cache_ttl.map(|ttl| {
            Cache::builder()
                .max_capacity(self.cache_size)
                .time_to_live(ttl)
                .build()
        });

        if self.endpoints.is_empty() {
            return Err(PersistedDocumentsError::MissingConfigurationOption(
                "endpoints".to_string(),
            ));
        }

        Ok(PersistedDocumentsManager {
            client,
            cache,
            negative_cache,
            endpoints_with_circuit_breakers: self
                .endpoints
                .into_iter()
                .map(move |endpoint| {
                    let circuit_breaker = self.circuit_breaker.clone().build_async()?;
                    Ok((endpoint, circuit_breaker))
                })
                .collect::<Result<Vec<(String, AsyncRecloser)>, PersistedDocumentsError>>()?,
        })
    }
}
