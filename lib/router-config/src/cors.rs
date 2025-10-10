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

    /// List of allowed origins. If `allow_any_origin` is true, this field is ignored.
    /// If both `origins` and `match_origin` are set, the request origin must match one of the values in either list to be allowed.
    /// An origin is a combination of scheme, host, and port (if specified).
    /// Example: "https://example.com", "http://localhost:3000"
    pub origins: Option<Vec<String>>,

    /// List of regex patterns to match allowed origins. If `allow_any_origin` is true, this field is ignored.
    /// If both `origins` and `match_origin` are set, the request origin must match one of the values in either list to be allowed.
    /// Each pattern should be a valid regex.
    /// Example: "^https://.*\.example\.com$", "^http://localhost:\d+$"
    pub match_origin: Option<Vec<String>>,

    /// Set to true to allow credentials (cookies, authorization headers, or TLS client certificates) in cross-origin requests.
    /// This will set the `Access-Control-Allow-Credentials` header to `true`.
    pub allow_credentials: bool,

    /// List of headers that the server allows the client to send in a cross-origin request.
    /// This will set the `Access-Control-Allow-Headers` header.
    /// If not set, the server will reflect the headers specified in the `Access-Control-Request-Headers` request header.
    /// Example: ["Content-Type", "Authorization"]
    pub allow_headers: Option<Vec<String>>,

    /// List of methods that the server allows for cross-origin requests.
    /// This will set the `Access-Control-Allow-Methods` header.
    /// If not set, the server will reflect the method specified in the `Access-Control-Request-Method` request header.
    /// Example: ["GET", "POST", "OPTIONS"]
    pub methods: Option<Vec<String>>,

    /// List of headers that the client is allowed to access from the response.
    /// This will set the `Access-Control-Expose-Headers` header.
    /// If not set, no additional headers are exposed to the client.
    /// Example: ["X-Custom-Header", "X-Another-Header"]
    pub expose_headers: Option<Vec<String>>,

    /// The maximum time (in seconds) that the results of a preflight request can be cached by the client.
    /// This will set the `Access-Control-Max-Age` header.
    /// If not set, the browser will not cache the preflight response.
    /// Example: 86400 (24 hours)
    pub max_age: Option<u64>,
}

fn default_cors_enabled() -> bool {
    false
}

fn default_allow_any_origin() -> bool {
    false
}

fn cors_example_1() -> CORSConfig {
    CORSConfig {
        enabled: true,
        allow_any_origin: false,
        origins: Some(vec![
            "https://example.com".to_string(),
            "https://another.com".to_string(),
        ]),
        match_origin: None,
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
