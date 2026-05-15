use std::{sync::Arc, time::Duration};

use async_dropper_simple::AsyncDropper;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest_middleware::ClientBuilder;
use reqwest_retry::RetryTransientMiddleware;

use crate::agent::buffer::Buffer;
use crate::agent::config::UsageReportingConfig;
use crate::agent::exclude::Exclude;
use crate::agent::sampler::Sampler;
use crate::agent::usage_agent::{non_empty_string, AgentError, UsageAgent, UsageAgentInner};
use crate::agent::utils::OperationProcessor;
use crate::circuit_breaker::CircuitBreakerBuilder;
use crate::primitives::circuit_breaker::CircuitBreakerConfig;
use crate::primitives::target_id::TargetId;
use retry_policies::policies::ExponentialBackoff;

pub struct UsageAgentBuilder {
    token: Option<String>,
    endpoint: String,
    target_id: Option<TargetId>,
    buffer_size: usize,
    connect_timeout: Duration,
    request_timeout: Duration,
    accept_invalid_certs: bool,
    flush_interval: Duration,
    retry_policy: ExponentialBackoff,
    circuit_breaker: CircuitBreakerConfig,
    user_agent: Option<String>,
    exclude: Option<Exclude>,
    sampler: Option<Sampler>,
}

pub use crate::agent::config::DEFAULT_HIVE_USAGE_ENDPOINT;

impl Default for UsageAgentBuilder {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_HIVE_USAGE_ENDPOINT.to_string(),
            token: None,
            target_id: None,
            buffer_size: 1000,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(15),
            accept_invalid_certs: false,
            flush_interval: Duration::from_secs(5),
            retry_policy: ExponentialBackoff::builder().build_with_max_retries(3),
            circuit_breaker: CircuitBreakerConfig::default(),
            user_agent: None,
            exclude: None,
            sampler: None,
        }
    }
}

fn is_legacy_token(token: &str) -> bool {
    !token.starts_with("hvo1/") && !token.starts_with("hvu1/") && !token.starts_with("hvp1/")
}

impl UsageAgentBuilder {
    /// Apply all values from a [`UsageReportingConfig`] to this builder.
    ///
    /// Token, target id and `User-Agent` header are intentionally left out
    /// here, because they are not part of the deserialized config: they come
    /// from environment variables / runtime values resolved by the caller.
    /// Set them with [`Self::token`], [`Self::target_id`] and
    /// [`Self::user_agent`] before calling [`Self::build`].
    ///
    /// Any VRL expression on `sampler.key.expression` or `exclude.expression`
    /// is compiled here, so configuration errors (invalid VRL, etc.) surface
    /// synchronously when the agent is built rather than on the first sampled
    /// report.
    pub fn from_config(mut self, config: &UsageReportingConfig) -> Result<Self, AgentError> {
        self.endpoint = config.endpoint.clone();
        self.buffer_size = config.buffer_size;
        self.connect_timeout = config.connect_timeout;
        self.request_timeout = config.request_timeout;
        self.accept_invalid_certs = config.accept_invalid_certs;
        self.flush_interval = config.flush_interval;
        self.retry_policy = (&config.retry_policy).into();
        if let Some(cb) = &config.circuit_breaker {
            self.circuit_breaker = cb.clone();
        }

        if let Some(exclude) = &config.exclude {
            self.exclude = Some(Exclude::from_config(exclude)?);
        }

        self.sampler = Some(Sampler::from_config(&config.sampler)?);

        Ok(self)
    }
    /// Your [Registry Access Token](https://the-guild.dev/graphql/hive/docs/management/targets#registry-access-tokens) with write permission.
    pub fn token(mut self, token: String) -> Self {
        if let Some(token) = non_empty_string(Some(token)) {
            self.token = Some(token);
        }
        self
    }
    /// A validated target id ([slug or UUID](crate::primitives::target_id::TargetId)),
    /// to be used when the token is configured with an organization access token.
    pub fn target_id(mut self, target_id: TargetId) -> Self {
        self.target_id = Some(target_id);
        self
    }
    /// User-Agent header to be sent with each request.
    ///
    /// This is intentionally not part of [`UsageReportingConfig`] because it
    /// is set by the embedding router (e.g. `hive-router/X.Y.Z`) and never
    /// configured by the operator.
    pub fn user_agent(mut self, user_agent: String) -> Self {
        if let Some(user_agent) = non_empty_string(Some(user_agent)) {
            self.user_agent = Some(user_agent);
        }
        self
    }
    pub(crate) fn build_agent(self) -> Result<UsageAgentInner, AgentError> {
        let mut default_headers = HeaderMap::new();

        default_headers.insert("X-Usage-API-Version", HeaderValue::from_static("2"));

        let token = match self.token {
            Some(token) => token,
            None => return Err(AgentError::MissingToken),
        };

        let mut authorization_header = HeaderValue::from_str(&format!("Bearer {}", token))
            .map_err(|_| AgentError::InvalidToken)?;

        authorization_header.set_sensitive(true);

        default_headers.insert(reqwest::header::AUTHORIZATION, authorization_header);

        default_headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        let mut reqwest_agent = reqwest::Client::builder()
            .danger_accept_invalid_certs(self.accept_invalid_certs)
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .default_headers(default_headers);

        if let Some(user_agent) = &self.user_agent {
            reqwest_agent = reqwest_agent.user_agent(user_agent);
        }

        let reqwest_agent = reqwest_agent
            .build()
            .map_err(AgentError::HTTPClientCreationError)?;
        let client = ClientBuilder::new(reqwest_agent)
            .with(RetryTransientMiddleware::new_with_policy(self.retry_policy))
            .build();

        let mut endpoint = self.endpoint;

        match self.target_id {
            Some(_) if is_legacy_token(&token) => return Err(AgentError::TargetIdWithLegacyToken),
            Some(target_id) if !is_legacy_token(&token) => {
                endpoint.push_str(&format!("/{}", target_id));
            }
            None if !is_legacy_token(&token) => return Err(AgentError::MissingTargetId),
            _ => {}
        }

        let cb_builder: CircuitBreakerBuilder = (&self.circuit_breaker).into();
        let circuit_breaker = cb_builder
            .build_async()
            .map_err(AgentError::CircuitBreakerCreationError)?;

        let buffer = Buffer::new(self.buffer_size);

        Ok(UsageAgentInner {
            endpoint,
            buffer,
            processor: OperationProcessor::new(),
            client,
            flush_interval: self.flush_interval,
            circuit_breaker,
            exclude: self.exclude,
            sampler: self.sampler.unwrap_or_default(),
        })
    }
    pub fn build(self) -> Result<UsageAgent, AgentError> {
        let agent = self.build_agent()?;
        Ok(Arc::new(AsyncDropper::new(agent)))
    }
}
