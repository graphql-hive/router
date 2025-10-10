/// Configuration for CSRF prevention.
///
/// Cross-site request forgery (CSRF) is an attack that forces an end user to execute unwanted actions on a web application in which they're currently authenticated.
/// By enabling CSRF prevention, the router will check for the presence of specific headers in incoming requests to the `/graphql` endpoint.
/// If the required headers are not present, the router will reject the request with a `403 Forbidden` response.
/// This helps to ensure that requests are coming from trusted sources and not from malicious third-party sites.
///
/// When CSRF prevention is enabled, the router only executes operations if at least one of the following conditions is true;
///
/// - The incoming request includes a `Content-Type` header other than a value of
///   - `text/plain`
///   - `application/x-www-form-urlencoded`
///   - `multipart/form-data`
///
/// - The incoming request includes at least one of the headers specified in the `required_headers` configuration.
///
/// ## Case sensitivity
/// Header names are case-insensitive, so `X-CSRF-Token` and `x-csrf-token` are treated the same.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone)]
#[schemars(example = csrf_prevention_example_1())]
pub struct CSRFPreventionConfig {
    #[serde(default)]
    /// A list of required header names for CSRF protection.
    pub required_headers: Vec<String>,
}

fn csrf_prevention_example_1() -> CSRFPreventionConfig {
    CSRFPreventionConfig {
        required_headers: vec!["x-csrf-token".to_string()],
    }
}
