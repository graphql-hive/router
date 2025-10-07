use hive_router_config::csrf::CSRFPreventionConfig;
use ntex::web::HttpRequest;

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};

// NON_PREFLIGHTED_CONTENT_TYPES are content types that do not require a preflight
// OPTIONS request. These are content types that are considered "simple" by the CORS
// specification.
// See: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS#simple_requests
const NON_PREFLIGHTED_CONTENT_TYPES: [&str; 3] = [
    "application/x-www-form-urlencoded",
    "multipart/form-data",
    "text/plain",
];

pub fn perform_csrf_prevention(
    req: &mut HttpRequest,
    csrf_config: &CSRFPreventionConfig,
) -> Result<(), PipelineError> {
    // If CSRF prevention is not configured, skip the checks.
    if csrf_config.required_headers.is_empty() {
        return Ok(());
    }

    if was_the_request_already_preflight_checked(req) {
        return Ok(());
    }

    // Check for the presence of at least one required header.
    let has_required_header = csrf_config.required_headers.iter().any(|header_name| {
        req.headers()
            .keys()
            .any(|h| h.as_str().eq_ignore_ascii_case(header_name))
    });

    if has_required_header {
        Ok(())
    } else {
        Err(req.new_pipeline_error(PipelineErrorVariant::CsrfPreventionFailed))
    }
}

fn was_the_request_already_preflight_checked(req: &HttpRequest) -> bool {
    match req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|ct| ct.to_str().ok())
    {
        Some(content_type) => !NON_PREFLIGHTED_CONTENT_TYPES.iter().any(|&non_prefetched| {
            content_type
                .to_ascii_lowercase()
                .starts_with(non_prefetched)
        }),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn do_not_allow_requests_without_the_necessary_header() {
        let config = super::CSRFPreventionConfig {
            required_headers: vec!["x-csrf-token".to_string()],
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
            required_headers: vec!["x-csrf-token".to_string()],
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
            required_headers: vec!["x-csrf-token".to_string()],
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
            required_headers: vec!["x-csrf-token".to_string()],
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
            required_headers: vec!["x-csrf-token".to_string()],
        };
        let mut req = ntex::web::test::TestRequest::with_uri("/graphql")
            .method(http::Method::POST)
            .header("Content-Type", "multipart/form-data; boundary=something")
            .to_http_request();
        let result = super::perform_csrf_prevention(&mut req, &config);
        assert!(result.is_err());
    }
}
