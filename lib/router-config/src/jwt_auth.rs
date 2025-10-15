use std::time::Duration;

use jsonwebtoken::Algorithm;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::primitives::{file_path::FilePath, http_header::HttpHeaderName};

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub struct JwtAuthConfig {
    /// A list of JWKS providers to use for verifying the JWT signature.
    /// Can be either a path to a local JSON of the file-system, or a URL to a remote JWKS provider.
    pub jwks_providers: Vec<JwksProviderSourceConfig>,
    /// Specify the [principal](https://tools.ietf.org/html/rfc7519#section-4.1.1) that issued the JWT, usually a URL or an email address.
    /// If specified, it has to match the `iss` field in JWT, otherwise the token's `iss` field is not checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuers: Option<Vec<String>>,
    /// The list of [JWT audiences](https://tools.ietf.org/html/rfc7519#section-4.1.3) are allowed to access.
    /// If this field is set, the token's `aud` field must be one of the values in this list, otherwise the token's `aud` field is not checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audiences: Option<Vec<String>>,
    /// A list of locations to look up for the JWT token in the incoming HTTP request.
    /// The first one that is found will be used.
    #[serde(
        default = "default_lookup_location",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub lookup_locations: Vec<JwtAuthPluginLookupLocation>,
    /// If set to `true`, the entire request will be rejected if the JWT token is not present in the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_authentication: Option<bool>,
    /// List of allowed algorithms for verifying the JWT signature.
    /// If not specified, the default list of all supported algorithms in [`jsonwebtoken` crate](https://crates.io/crates/jsonwebtoken) are used.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default = "default_allowed_algorithms"
    )]
    #[schemars(with = "Option<Vec<String>>")]
    pub allowed_algorithms: Option<Vec<Algorithm>>,
    #[serde(default = "default_forward_claims_to_upstream_extensions")]
    /// Forward the JWT claims to the upstream service using GraphQL's `.extensions`.
    pub forward_claims_to_upstream_extensions: JwtClaimsForwardingConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct JwtClaimsForwardingConfig {
    pub enabled: bool,
    pub field_name: String,
}

fn default_forward_claims_to_upstream_extensions() -> JwtClaimsForwardingConfig {
    JwtClaimsForwardingConfig {
        enabled: false,
        field_name: "jwt".to_string(),
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
#[serde(tag = "source")]
pub enum JwksProviderSourceConfig {
    /// A local file on the file-system. This file will be read once on startup and cached.
    #[serde(rename = "file")]
    #[schemars(title = "file")]
    File {
        #[serde(rename = "path")]
        /// A path to a local file on the file-system. Relative to the location of the root configuration file.
        file: FilePath,
    },
    /// A remote JWKS provider. The JWKS will be fetched via HTTP/HTTPS and cached.
    #[serde(rename = "remote")]
    #[schemars(title = "remote")]
    Remote {
        /// The URL to fetch the JWKS key set from, via HTTP/HTTPS.
        url: String,
        #[serde(
            deserialize_with = "humantime_serde::deserialize",
            serialize_with = "humantime_serde::serialize",
            default = "default_polling_interval"
        )]
        #[schemars(with = "String")]
        /// How often the JWKS should be polled for updates.
        polling_interval: Option<Duration>,
        /// If set to `true`, the JWKS will be fetched on startup and cached. In case of invalid JWKS, the error will be ignored and the plugin will try to fetch again when server receives the first request.
        /// If set to `false`, the JWKS will be fetched on-demand, when the first request comes in.
        prefetch: Option<bool>,
    },
}

fn default_polling_interval() -> Option<Duration> {
    // Some providers like MS Azure have rate limit configured. So let's use 10 minutes, like Envoy does.
    // and allow users to adjust it if needed.
    // See https://community.auth0.com/t/caching-jwks-signing-key/17654/2
    Some(Duration::from_secs(10 * 60))
}

pub fn default_lookup_location() -> Vec<JwtAuthPluginLookupLocation> {
    vec![JwtAuthPluginLookupLocation::Header {
        name: "Authorization".into(),
        prefix: Some("Bearer".to_string()),
    }]
}

pub fn default_allowed_algorithms() -> Option<Vec<Algorithm>> {
    Some(vec![
        Algorithm::HS256,
        Algorithm::HS384,
        Algorithm::HS512,
        Algorithm::RS256,
        Algorithm::RS384,
        Algorithm::RS512,
        Algorithm::ES256,
        Algorithm::ES384,
        Algorithm::PS256,
        Algorithm::PS384,
        Algorithm::PS512,
        Algorithm::EdDSA,
    ])
}

#[derive(Deserialize, Serialize, Debug, Clone, JsonSchema)]
#[serde(tag = "source")]
pub enum JwtAuthPluginLookupLocation {
    #[serde(rename = "header")]
    #[schemars(title = "header")]
    Header {
        name: HttpHeaderName,
        prefix: Option<String>,
    },
    #[serde(rename = "cookies")]
    #[schemars(title = "cookies")]
    Cookie { name: String },
}
