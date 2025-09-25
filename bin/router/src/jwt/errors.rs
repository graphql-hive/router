use hive_router_plan_executor::response::graphql_error::GraphQLError;
use http::{
    header::{InvalidHeaderValue, ToStrError},
    StatusCode,
};
use ntex::{http::ResponseBuilder, web};

use crate::pipeline::error::FailedExecutionResult;

#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    #[error("failed to locate the value in the incoming request")]
    LookupFailed,
    #[error("prefix does not match the found value")]
    MismatchedPrefix,
    #[error("failed to convert header to string")]
    FailedToStringifyHeader(ToStrError),
    #[error("failed to parse header value")]
    FailedToParseHeader(InvalidHeaderValue),
}

#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    #[error("jwt header lookup failed: {0}")]
    LookupFailed(LookupError),
    #[error("failed to parse JWT header: {0}")]
    InvalidJwtHeader(jsonwebtoken::errors::Error),
    #[error("failed to decode JWK: {0}")]
    InvalidDecodingKey(jsonwebtoken::errors::Error),
    #[error("token is not supported by any of the configured providers")]
    FailedToLocateProvider,
    #[error("failed to locate algorithm in jwk")]
    JwkMissingAlgorithm,
    #[error("jwk algorithm is not supported: {0}")]
    JwkAlgorithmNotSupported(jsonwebtoken::errors::Error),
    #[error("failed to decode token: {0}")]
    FailedToDecodeToken(jsonwebtoken::errors::Error),
    #[error("all jwk failed to decode token: {0:?}")]
    AllProvidersFailedToDecode(Vec<JwtError>),
    #[error("http request parsing error: {0:?}")]
    HTTPRequestParsingError(String),
}

impl JwtError {
    pub fn into_response(&self) -> web::HttpResponse {
        let status_code: StatusCode = self.into();
        let validation_error_result = FailedExecutionResult {
            errors: Some(vec![GraphQLError {
                extensions: None,
                message: self.to_string(),
                locations: None,
                path: None,
            }]),
        };

        return ResponseBuilder::new(status_code).json(&validation_error_result);
    }
}

impl From<&JwtError> for StatusCode {
    fn from(val: &JwtError) -> Self {
        match val {
            JwtError::InvalidJwtHeader(_)
            | JwtError::LookupFailed(_)
            | JwtError::JwkAlgorithmNotSupported(_)
            | JwtError::HTTPRequestParsingError(_) => StatusCode::BAD_REQUEST,
            JwtError::JwkMissingAlgorithm
            | JwtError::FailedToLocateProvider
            | JwtError::InvalidDecodingKey(_) => StatusCode::INTERNAL_SERVER_ERROR,
            JwtError::AllProvidersFailedToDecode(_) | JwtError::FailedToDecodeToken(_) => {
                StatusCode::UNAUTHORIZED
            }
        }
    }
}
