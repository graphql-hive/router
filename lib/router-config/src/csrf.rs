use crate::primitives::http_header::HttpHeaderName;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for CSRF prevention.
///
/// Cross-site request forgery (CSRF) is an attack that forces an end user to execute unwanted actions on a web application in which they're currently authenticated.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[schemars(example = csrf_prevention_example_1())]
pub struct CSRFPreventionConfig {
    /// Enables CSRF prevention.
    ///
    /// By enabling CSRF prevention, the router will check for the presence of specific headers in incoming requests to the `/graphql` endpoint.
    /// If the required headers are not present, the router will reject the request with a `403 Forbidden` response.
    /// This triggers the preflight checks in browsers, preventing the request from being sent.
    /// So you can ensure that only requests from trusted origins are processed.
    ///
    /// When CSRF prevention is enabled, the router only executes operations if one of the following conditions is true;
    ///
    /// - The incoming request includes a `Content-Type` header other than a value of
    ///   - `text/plain`
    ///   - `application/x-www-form-urlencoded`
    ///   - `multipart/form-data`
    ///
    /// - The incoming request includes at least one of the headers specified in the `required_headers` configuration.
    #[serde(default = "default_csrf_enabled")]
    pub enabled: bool,

    #[serde(default)]
    /// A list of required header names for CSRF protection.
    ///
    /// Header names are case-insensitive.
    pub required_headers: Vec<HttpHeaderName>,
}

fn csrf_prevention_example_1() -> CSRFPreventionConfig {
    CSRFPreventionConfig {
        enabled: true,
        required_headers: vec!["x-csrf-token".into()],
    }
}

fn default_csrf_enabled() -> bool {
    true
}
