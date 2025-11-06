use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AwsSigV4Config {
    /// Enables or disables AWS Signature Version 4 signing for requests to subgraphs.
    /// When enabled, the router will sign requests to subgraphs using AWS SigV4.
    #[serde(default = "default_aws_sig_v4_enabled")]
    pub enabled: bool,

    // configuration that will apply to all subgraphs
    pub all: AwsSigV4SubgraphConfig,

    // per-subgraph configuration overrides
    #[serde(default)]
    pub subgraphs: HashMap<String, AwsSigV4SubgraphConfig>,
}

impl Default for AwsSigV4Config {
    fn default() -> Self {
        Self {
            enabled: default_aws_sig_v4_enabled(),
            all: default_all_config(),
            subgraphs: HashMap::new(),
        }
    }
}

fn default_all_config() -> AwsSigV4SubgraphConfig {
    AwsSigV4SubgraphConfig::DefaultChain(DefaultChainConfig {
        profile_name: None,
        region: None,
        service: None,
        assume_role: None,
    })
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum AwsSigV4SubgraphConfig {
    DefaultChain(DefaultChainConfig),
    // Not recommended, prefer using default_chain as shown above
    HardCoded(HardCodedConfig),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DefaultChainConfig {
    // https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/iam-roles-for-amazon-ec2.html#ec2-instance-profile
    pub profile_name: Option<String>,

    // https://docs.aws.amazon.com/general/latest/gr/rande.html
    pub region: Option<String>,

    // https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_aws-services-that-work-with-iam.html
    pub service: Option<String>,

    pub assume_role: Option<AssumeRoleConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct HardCodedConfig {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
    pub service_name: String,
    pub session_token: Option<String>,
}

impl AwsSigV4Config {
    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }
}

fn default_aws_sig_v4_enabled() -> bool {
    false
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AssumeRoleConfig {
    pub role_arn: String,
    pub session_name: Option<String>,
    pub external_id: Option<String>,
}
