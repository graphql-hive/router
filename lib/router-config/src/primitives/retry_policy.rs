use retry_policies::policies::ExponentialBackoff as RetryPoliciesExponentialBackoff;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RetryPolicyConfig {
    /// The maximum number of retries to attempt.
    ///
    /// Retry mechanism is based on exponential backoff, see https://docs.rs/retry-policies/latest/retry_policies/policies/struct.ExponentialBackoff.html for additional details.
    pub max_retries: u32,
}

impl From<&RetryPolicyConfig> for RetryPoliciesExponentialBackoff {
    fn from(config: &RetryPolicyConfig) -> Self {
        RetryPoliciesExponentialBackoff::builder().build_with_max_retries(config.max_retries)
    }
}
