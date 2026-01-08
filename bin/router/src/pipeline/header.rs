use http::{
    header::{ACCEPT, CONTENT_TYPE},
    HeaderValue,
};
use lazy_static::lazy_static;
use ntex::web::HttpRequest;
use tracing::{trace, warn};

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};

lazy_static! {
    pub static ref APPLICATION_JSON_STR: &'static str = "application/json";
    pub static ref APPLICATION_JSON: HeaderValue = HeaderValue::from_static(&APPLICATION_JSON_STR);
    pub static ref APPLICATION_GRAPHQL_RESPONSE_JSON_STR: &'static str =
        "application/graphql-response+json";
    pub static ref TEXT_HTML_CONTENT_TYPE: &'static str = "text/html";
    pub static ref APPLICATION_GRAPHQL_RESPONSE_JSON: HeaderValue =
        HeaderValue::from_static(&APPLICATION_GRAPHQL_RESPONSE_JSON_STR);
    pub static ref TEXT_EVENT_STREAM: &'static str = "text/event-stream";
    pub static ref MULTIPART_MIXED: &'static str = "multipart/mixed";
}

pub trait RequestAccepts {
    /// Checks if the request's `Accept` header contains the given content type.
    /// If `suffix` is provided, it ensures that the content type is accompanied by
    /// the suffix like when matching exactly `Accept: multipart/mixed; spec="1.0"`
    /// with `content_type = "multipart/mixed"` and `suffix = Some("spec="1.0")`.
    /// When providing the suffix, the content-type and suffix must be in the same part of the
    /// `Accept` header (comma-separated).
    fn accepts_content_type(&self, content_type: &str, suffix: Option<&str>) -> bool;
}

impl RequestAccepts for HttpRequest {
    #[inline]
    fn accepts_content_type(&self, content_type: &str, suffix: Option<&str>) -> bool {
        self.headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .map(|s| match suffix {
                None => s.contains(content_type),
                // suffix needs to be alongside the content type itself, so we must split ","
                // to avoid false positives like when checking `(multipart/mixed, spec="1.0")` with:
                // `accept: multipart/mixed, text/event-stream;spec="1.0"`
                Some(suffix) => s
                    .split(',')
                    .map(|part| part.trim())
                    .any(|part| part.contains(content_type) && part.contains(suffix)),
            })
            .unwrap_or(false)
    }
}

pub trait AssertRequestJson {
    fn assert_json_content_type(&self) -> Result<(), PipelineError>;
}

impl AssertRequestJson for HttpRequest {
    #[inline]
    fn assert_json_content_type(&self) -> Result<(), PipelineError> {
        match self.headers().get(CONTENT_TYPE) {
            Some(value) => {
                let content_type_str = value.to_str().map_err(|_| {
                    self.new_pipeline_error(PipelineErrorVariant::InvalidHeaderValue(CONTENT_TYPE))
                })?;
                if !content_type_str.contains(*APPLICATION_JSON_STR) {
                    warn!(
                        "Invalid content type on a POST request: {}",
                        content_type_str
                    );
                    return Err(
                        self.new_pipeline_error(PipelineErrorVariant::UnsupportedContentType)
                    );
                }
                Ok(())
            }
            None => {
                trace!("POST without content type detected");
                Err(self.new_pipeline_error(PipelineErrorVariant::MissingContentTypeHeader))
            }
        }
    }
}
