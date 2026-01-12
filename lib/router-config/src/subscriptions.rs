use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionsConfig {
    /// Enables/disables subscriptions. By default, the subscriptions are disabled.
    ///
    /// You can override this setting by setting the `SUBSCRIPTIONS_ENABLED` environment variable to `true` or `false`.
    #[serde(default = "default_subscriptions_enabled")]
    pub enabled: bool,
}

fn default_subscriptions_enabled() -> bool {
    false
}
