use std::vec;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for CORS (Cross-Origin Resource Sharing).
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[schemars(example = cors_example_1())]
pub struct CORSConfig {
    #[serde(default = "default_cors_enabled")]
    pub enabled: bool,

    /// Set to true to allow any origin. If true, the `origins` and `match_origin` fields are ignored.
    #[serde(default = "default_allow_any_origin")]
    pub allow_any_origin: bool,

    /// List of CORS policies. The first policy that matches the request origin will be applied.
    /// If no policies match, the request will be rejected.
    /// If `allow_any_origin` is true, this field is ignored.
    /// This allows you to define different CORS settings for different origins.
    /// For example, you might want to allow credentials for some origins but not others.
    /// If multiple policies match, the first one in the list will be applied.
    ///
    /// Example:
    /// ```yaml
    /// allow_credentials: false
    /// policies:
    ///   - match_origin: ["^https://.*\.credentials-example\.com$"]
    ///     allow_credentials: true
    ///   - match_origin: ["^https://.*\.example\.com$"]
    /// ```
    ///
    /// In this example, requests from any subdomain of `credentials-example.com` will be allowed to include credentials,
    /// while requests from any subdomain of `example.com` will not be allowed to include credentials.
    /// Requests from origins not matching either pattern will be rejected.
    pub policies: Vec<CORSPolicyConfig>,

    /// Set to true to allow credentials (cookies, authorization headers, or TLS client certificates) in cross-origin requests.
    /// This will set the `Access-Control-Allow-Credentials` header to `true`.
    #[serde(default = "default_allow_credentials")]
    pub allow_credentials: bool,

    /// List of headers that the server allows the client to send in a cross-origin request.
    /// This will set the `Access-Control-Allow-Headers` header.
    /// If not set, the server will reflect the headers specified in the `Access-Control-Request-Headers` request header.
    /// Example: ["Content-Type", "Authorization"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_headers: Option<Vec<String>>,

    /// List of methods that the server allows for cross-origin requests.
    /// This will set the `Access-Control-Allow-Methods` header.
    /// If not set, the server will reflect the method specified in the `Access-Control-Request-Method` request header.
    /// Example: ["GET", "POST", "OPTIONS"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub methods: Option<Vec<String>>,

    /// List of headers that the client is allowed to access from the response.
    /// This will set the `Access-Control-Expose-Headers` header.
    /// If not set, no additional headers are exposed to the client.
    /// Example: ["X-Custom-Header", "X-Another-Header"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expose_headers: Option<Vec<String>>,

    /// The maximum time (in seconds) that the results of a preflight request can be cached by the client.
    /// This will set the `Access-Control-Max-Age` header.
    /// If not set, the browser will not cache the preflight response.
    /// Example: 86400 (24 hours)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age: Option<u64>,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
pub struct CORSPolicyConfig {
    /// List of allowed origins. If `allow_any_origin` is true, this field is ignored.
    /// If both `origins` and `match_origin` are set, the request origin must match one of the values in either list to be allowed.
    /// An origin is a combination of scheme, host, and port (if specified).
    /// Example: "https://example.com", "http://localhost:3000"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origins: Option<Vec<String>>,

    /// List of regex patterns to match allowed origins. If `allow_any_origin` is true, this field is ignored.
    /// If both `origins` and `match_origin` are set, the request origin must match one of the values in either list to be allowed.
    /// Each pattern should be a valid regex.
    /// Example: "^https://.*\.example\.com$", "^http://localhost:\d+$"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_origin: Option<Vec<String>>,

    /// Set to true to allow credentials (cookies, authorization headers, or TLS client certificates) in cross-origin requests.
    /// This will set the `Access-Control-Allow-Credentials` header to `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_credentials: Option<bool>,

    /// List of headers that the server allows the client to send in a cross-origin request.
    /// This will set the `Access-Control-Allow-Headers` header.
    /// If not set, the server will reflect the headers specified in the `Access-Control-Request-Headers` request header.
    /// Example: ["Content-Type", "Authorization"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_headers: Option<Vec<String>>,

    /// List of methods that the server allows for cross-origin requests.
    /// This will set the `Access-Control-Allow-Methods` header.
    /// If not set, the server will reflect the method specified in the `Access-Control-Request-Method` request header.
    /// Example: ["GET", "POST", "OPTIONS"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub methods: Option<Vec<String>>,

    /// List of headers that the client is allowed to access from the response.
    /// This will set the `Access-Control-Expose-Headers` header.
    /// If not set, no additional headers are exposed to the client.
    /// Example: ["X-Custom-Header", "X-Another-Header"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expose_headers: Option<Vec<String>>,

    /// The maximum time (in seconds) that the results of a preflight request can be cached by the client.
    /// This will set the `Access-Control-Max-Age` header.
    /// If not set, the browser will not cache the preflight response.
    /// Example: 86400 (24 hours)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age: Option<u64>,
}

fn default_cors_enabled() -> bool {
    false
}

fn default_allow_any_origin() -> bool {
    false
}

fn default_allow_credentials() -> bool {
    false
}

fn cors_example_1() -> CORSConfig {
    CORSConfig {
        enabled: true,
        allow_any_origin: false,
        policies: vec![CORSPolicyConfig {
            origins: Some(vec![
                "https://example.com".to_string(),
                "https://another.com".to_string(),
            ]),
            ..Default::default()
        }],
        allow_credentials: false,
        allow_headers: None,
        methods: Some(vec![
            "GET".to_string(),
            "POST".to_string(),
            "OPTIONS".to_string(),
        ]),
        expose_headers: None,
        max_age: Some(120),
    }
}
