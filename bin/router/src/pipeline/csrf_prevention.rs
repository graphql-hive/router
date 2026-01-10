use hive_router_config::csrf::CSRFPreventionConfig;
use ntex::web::HttpRequest;

use crate::pipeline::error::PipelineErrorVariant;

// NON_PREFLIGHTED_CONTENT_TYPES are content types that do not require a preflight
// OPTIONS request. These are content types that are considered "simple" by the CORS
// specification.
// See: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS#simple_requests
const NON_PREFLIGHTED_CONTENT_TYPES: [&str; 3] = [
    "application/x-www-form-urlencoded",
    "multipart/form-data",
    "text/plain",
];

#[inline]
pub fn perform_csrf_prevention(
    req: &HttpRequest,
    csrf_config: &CSRFPreventionConfig,
) -> Result<(), PipelineErrorVariant> {
    // If CSRF prevention is not configured or disabled, skip the checks.
    if !csrf_config.enabled || csrf_config.required_headers.is_empty() {
        return Ok(());
    }

    // If the request is considered preflighted, skip the check
    if request_requires_preflight(req) {
        return Ok(());
    }

    // Check for the presence of at least one required header.
    // Requiring any headers others than the Content-Type header
    // forces browsers to preflight check the request.
    let has_required_header = csrf_config
        .required_headers
        .iter()
        .any(|header_name| req.headers().contains_key(header_name.get_header_ref()));

    if has_required_header {
        Ok(())
    } else {
        Err(PipelineErrorVariant::CsrfPreventionFailed)
    }
}

/// A content type is considered "simple" if it does not trigger a CORS preflight.
/// See: https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/CORS#preflighted_requests
fn is_simple_content_type(content_type: &str) -> bool {
    let lowercased_content_type = content_type.to_ascii_lowercase();
    NON_PREFLIGHTED_CONTENT_TYPES
        .iter()
        .any(|&simple_type| lowercased_content_type.starts_with(simple_type))
}

/// Determines if the request was already preflight checked by looking at the Content-Type header.
/// If the Content-Type is not one of the NON_PREFLIGHTED_CONTENT_TYPES, we assume it was preflight checked.
fn request_requires_preflight(req: &HttpRequest) -> bool {
    req.headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| !is_simple_content_type(content_type))
}

#[cfg(test)]
mod tests {
    #[test]
    fn do_not_allow_requests_without_the_necessary_header() {
        let config = super::CSRFPreventionConfig {
            enabled: true,
            required_headers: vec!["x-csrf-token".into()],
        };
        let mut req = ntex::web::test::TestRequest::with_uri("/graphql")
            .method(http::Method::GET)
            .header("x-not-the-required", "header")
            .to_http_request();
        let result = super::perform_csrf_prevention(&mut req, &config);
        assert!(result.is_err());
    }

    #[test]
    fn allow_requests_with_necessary_header() {
        let config = super::CSRFPreventionConfig {
            enabled: true,
            required_headers: vec!["x-csrf-token".into()],
        };
        let mut req = ntex::web::test::TestRequest::with_uri("/graphql")
            .method(http::Method::GET)
            .header("x-csrf-token", "header")
            .to_http_request();
        let result = super::perform_csrf_prevention(&mut req, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn allow_post_requests_with_application_json_content_type() {
        let config = super::CSRFPreventionConfig {
            enabled: true,
            required_headers: vec!["x-csrf-token".into()],
        };
        let mut req = ntex::web::test::TestRequest::with_uri("/graphql")
            .method(http::Method::POST)
            .header("Content-Type", "application/json")
            .to_http_request();
        let result = super::perform_csrf_prevention(&mut req, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn allow_post_multipart_requests_with_necessary_header() {
        let config = super::CSRFPreventionConfig {
            enabled: true,
            required_headers: vec!["x-csrf-token".into()],
        };
        let mut req = ntex::web::test::TestRequest::with_uri("/graphql")
            .method(http::Method::POST)
            .header("x-csrf-token", "header")
            .header("Content-Type", "multipart/form-data; boundary=something")
            .to_http_request();
        let result = super::perform_csrf_prevention(&mut req, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn do_not_allow_post_multipart_requests_without_necessary_header() {
        let config = super::CSRFPreventionConfig {
            enabled: true,
            required_headers: vec!["x-csrf-token".into()],
        };
        let mut req = ntex::web::test::TestRequest::with_uri("/graphql")
            .method(http::Method::POST)
            .header("Content-Type", "multipart/form-data; boundary=something")
            .to_http_request();
        let result = super::perform_csrf_prevention(&mut req, &config);
        assert!(result.is_err());
    }

    #[test]
    fn case_insensitive_header_names() {
        let config = super::CSRFPreventionConfig {
            enabled: true,
            required_headers: vec!["x-csRf-token".into()],
        };
        let mut req = ntex::web::test::TestRequest::with_uri("/graphql")
            .method(http::Method::GET)
            .header("X-CSrF-ToKEN", "header")
            .to_http_request();
        let result = super::perform_csrf_prevention(&mut req, &config);
        assert!(result.is_ok());
    }
}
