use http::StatusCode;
use ntex::http::header::{InvalidHeaderValue, ToStrError};

#[derive(Debug, thiserror::Error, Clone)]
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

#[derive(Debug, thiserror::Error, Clone)]
pub enum JwtError {
    #[error("jwt header lookup failed: {0}")]
    LookupFailed(LookupError),
    #[error("failed to parse JWT header: {0}")]
    InvalidJwtHeader(jsonwebtoken::errors::Error),
    #[error("failed to decode JWK: {0}")]
    InvalidDecodingKey(jsonwebtoken::errors::Error),
    #[error("token is not supported by any of the configured providers")]
    FailedToLocateProvider,
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
    pub fn status_code(&self) -> StatusCode {
        match self {
            JwtError::LookupFailed(_) => StatusCode::UNAUTHORIZED,
            JwtError::JwkAlgorithmNotSupported(_) | JwtError::HTTPRequestParsingError(_) => {
                StatusCode::BAD_REQUEST
            }
            JwtError::AllProvidersFailedToDecode(_)
            | JwtError::InvalidJwtHeader(_)
            | JwtError::InvalidDecodingKey(_)
            | JwtError::FailedToLocateProvider
            | JwtError::FailedToDecodeToken(_) => StatusCode::FORBIDDEN,
        }
    }
    pub fn error_code(&self) -> &'static str {
        match self {
            JwtError::AllProvidersFailedToDecode(_) => "MISSING_JWT",
            JwtError::FailedToDecodeToken(_) => "INVALID_JWT",
            JwtError::FailedToLocateProvider => "JWT_NOT_SUPPORTED",
            JwtError::HTTPRequestParsingError(_) => "FAILED_TO_PARSE_REQUEST",
            JwtError::InvalidJwtHeader(_) => "INVALID_JWT_HEADER",
            JwtError::InvalidDecodingKey(_) => "INTERNAL_SERVER_ERROR",
            JwtError::JwkAlgorithmNotSupported(_) => "JWK_ALGORITHM_NOT_SUPPORTED",
            JwtError::LookupFailed(_) => "JWT_LOOKUP_FAILED",
        }
    }
}
