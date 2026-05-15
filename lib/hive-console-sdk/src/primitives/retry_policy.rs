use retry_policies::policies::ExponentialBackoff as RetryPoliciesExponentialBackoff;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration shared by every component that performs HTTP retries
/// against a Hive Console / Hive Registry endpoint (usage reporting,
/// persisted documents, supergraph fetcher, ...).
///
/// The retry mechanism is exponential backoff. See
/// <https://docs.rs/retry-policies/latest/retry_policies/policies/struct.ExponentialBackoff.html>
/// for the underlying policy.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RetryPolicyConfig {
    /// The maximum number of retries to attempt.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

impl Default for RetryPolicyConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
        }
    }
}

fn default_max_retries() -> u32 {
    3
}

impl From<&RetryPolicyConfig> for RetryPoliciesExponentialBackoff {
    fn from(config: &RetryPolicyConfig) -> Self {
        RetryPoliciesExponentialBackoff::builder().build_with_max_retries(config.max_retries)
    }
}
