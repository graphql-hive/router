use http::{
    header::{ACCEPT, CONTENT_TYPE},
    HeaderValue,
};
use lazy_static::lazy_static;
use tracing::{trace, warn};

use crate::pipeline::error::{PipelineError, PipelineErrorFromAcceptHeader, PipelineErrorVariant};

lazy_static! {
    pub static ref APPLICATION_JSON_STR: &'static str = "application/json";
    pub static ref APPLICATION_JSON: HeaderValue = HeaderValue::from_static(&APPLICATION_JSON_STR);
    pub static ref APPLICATION_GRAPHQL_RESPONSE_JSON_STR: &'static str =
        "application/graphql-response+json";
    pub static ref APPLICATION_GRAPHQL_RESPONSE_JSON: HeaderValue =
        HeaderValue::from_static(&APPLICATION_GRAPHQL_RESPONSE_JSON_STR);
}

pub trait RequestAccepts {
    fn accepts_content_type(&self, content_type: &str) -> bool;
}

impl RequestAccepts for http::Request<axum::body::Body> {
    fn accepts_content_type(&self, content_type: &str) -> bool {
        let accept_header = self.headers().get(http::header::ACCEPT);
        if let Some(value) = accept_header {
            value
                .to_str()
                .map(|s| s.contains(content_type))
                .unwrap_or(false)
        } else {
            false
        }
    }
}

pub trait AssertRequestJson {
    fn assert_json_content_type(&self) -> Result<(), PipelineError>;
}

impl AssertRequestJson for http::Request<axum::body::Body> {
    fn assert_json_content_type(&self) -> Result<(), PipelineError> {
        let request_content_type: Option<&str> = match self.headers().get(CONTENT_TYPE) {
            None => None,
            Some(content_type) => {
                let value = content_type.to_str().map_err(|_| {
                    self.new_pipeline_error(PipelineErrorVariant::InvalidHeaderValue(ACCEPT))
                })?;

                Some(value)
            }
        };
        match request_content_type {
            None => {
                trace!("POST without content type detected");
                return Err(self.new_pipeline_error(PipelineErrorVariant::MissingContentTypeHeader));
            }
            Some(content_type) => {
                if !content_type.contains(*APPLICATION_JSON_STR) {
                    warn!("Invalid content type on a POST request: {}", content_type);

                    return Err(
                        self.new_pipeline_error(PipelineErrorVariant::UnsupportedContentType)
                    );
                }
            }
        }
        Ok(())
    }
}
