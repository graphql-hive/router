use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Default, JsonSchema)]
pub struct HMACSignatureConfig {
    // Whether to sign outgoing requests with HMAC signatures.
    // Can be a boolean or a VRL expression that evaluates to a boolean.
    // Example:
    // hmac_signature:
    //  enabled: true
    // or enable it conditionally based on the subgraph name:
    // hmac_signature:
    //  enabled: |
    //    if .subgraph.name == "users" {
    //      true
    //    } else {
    //      false
    //    }
    #[serde(default = "default_hmac_signature_enabled")]
    pub enabled: BooleanOrExpression,

    // The secret key used for HMAC signing and verification.
    // It should be a random, opaque string shared between the Hive Router and the subgraph services.
    pub secret: String,

    // The key name used in the extensions field of the outgoing requests to store the HMAC signature.
    #[serde(default = "default_extension_name")]
    pub extension_name: String,
}

fn default_extension_name() -> String {
    "hmac_signature".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum BooleanOrExpression {
    Boolean(bool),
    Expression { expression: String },
}

impl Default for BooleanOrExpression {
    fn default() -> Self {
        BooleanOrExpression::Boolean(false)
    }
}

impl HMACSignatureConfig {
    pub fn is_disabled(&self) -> bool {
        match &self.enabled {
            BooleanOrExpression::Boolean(b) => !*b,
            BooleanOrExpression::Expression { expression: _ } => {
                // If it's an expression, we consider it enabled for the purpose of this check.
                false
            }
        }
    }
}

fn default_hmac_signature_enabled() -> BooleanOrExpression {
    BooleanOrExpression::Boolean(false)
}
