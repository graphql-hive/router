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
    fn accepts_content_type(&self, content_type: &str) -> bool;
}

impl RequestAccepts for HttpRequest {
    #[inline]
    fn accepts_content_type(&self, content_type: &str) -> bool {
        self.headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .map(|s| s.contains(content_type))
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
