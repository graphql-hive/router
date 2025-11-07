use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AwsSigV4Config {
    // configuration that will apply to all subgraphs
    pub all: AwsSigV4SubgraphConfig,

    // per-subgraph configuration overrides
    #[serde(default)]
    pub subgraphs: HashMap<String, AwsSigV4SubgraphConfig>,
}

impl Default for AwsSigV4Config {
    fn default() -> Self {
        Self {
            all: default_all_config(),
            subgraphs: HashMap::new(),
        }
    }
}

fn default_all_config() -> AwsSigV4SubgraphConfig {
    AwsSigV4SubgraphConfig::Disabled
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum AwsSigV4SubgraphConfig {
    Disabled,
    DefaultChain(DefaultChainConfig),
    // Not recommended, prefer using default_chain as shown above
    HardCoded(HardCodedConfig),
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DefaultChainConfig {
    // https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/iam-roles-for-amazon-ec2.html#ec2-instance-profile
    pub profile_name: Option<String>,

    // https://docs.aws.amazon.com/general/latest/gr/rande.html
    pub region: String,

    // https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_aws-services-that-work-with-iam.html
    pub service: String,

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
        matches!(self.all, AwsSigV4SubgraphConfig::Disabled) && self.subgraphs.is_empty()
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AssumeRoleConfig {
    pub role_arn: String,
    pub session_name: Option<String>,
}
