use hive_router_config::coprocessor::CoprocessorProtocol;
use hive_router_internal::expressions::ExpressionCompileError;
use http::header::ToStrError;
use http::uri::InvalidUri;
use ntex::http::error::PayloadError;

#[derive(thiserror::Error, Debug)]
pub enum CoprocessorError {
    #[error("coprocessor protocol '{0:?}' is not supported")]
    UnsupportedProtocol(CoprocessorProtocol),

    #[error("coprocessor unix:// request path must start with '/', received '{0}'")]
    InvalidUnixRequestPath(String),

    #[error("failed to parse coprocessor endpoint URI '{0}': {1}")]
    EndpointParseFailure(String, InvalidUri),

    #[error("failed to build coprocessor request: {0}")]
    RequestBuildFailure(#[source] http::Error),

    #[error("coprocessor request execution failed: {0}")]
    RequestExecutionFailure(#[source] hyper_util::client::legacy::Error),

    #[error("coprocessor returned non-success status: {0}")]
    UnexpectedStatus(http::StatusCode),

    #[error("coprocessor request to '{endpoint}' timed out after {timeout_ms}ms")]
    RequestTimeout { endpoint: String, timeout_ms: u128 },

    #[error("failed reading coprocessor response body: {0}")]
    ResponseBodyReadFailure(#[source] hyper::Error),

    #[error("invalid coprocessor content-encoding header: {0}")]
    InvalidContentEncodingHeader(#[source] ToStrError),

    #[error("unsupported stacked content-encoding from coprocessor: '{0}'")]
    UnsupportedStackedContentEncoding(String),

    #[error("unsupported content-encoding from coprocessor: '{0}'")]
    UnsupportedContentEncoding(String),

    #[error("failed to decompress coprocessor response using '{encoding}': {source}")]
    ResponseDecompressionFailure {
        encoding: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to compile coprocessor condition expression: {0}")]
    ConditionCompile(#[from] ExpressionCompileError),

    #[error("failed to evaluate coprocessor condition: {0}")]
    ConditionEvaluation(String),

    #[error("failed to read router request body for coprocessor: {0}")]
    RequestBodyRead(#[from] PayloadError),

    #[error("invalid UTF-8 body bytes in {context}: {source}")]
    InvalidUtf8Body {
        context: &'static str,
        #[source]
        source: std::str::Utf8Error,
    },

    #[error("failed to deserialize coprocessor response payload: {0}")]
    ResponseDeserialize(#[from] sonic_rs::Error),

    #[error("coprocessor returned unsupported version {0}")]
    UnsupportedVersion(u8),

    #[error("invalid HTTP header name in coprocessor payload: {0}")]
    InvalidHeaderName(String),

    #[error("invalid HTTP header value in coprocessor payload: {0}")]
    InvalidHeaderValue(String),

    #[error("invalid HTTP method in coprocessor payload: {0}")]
    InvalidMethod(String),

    #[error("invalid request path in coprocessor payload: {0}")]
    InvalidPath(String),

    #[error("coprocessor {stage} stage cannot mutate '{field}'")]
    ForbiddenStageMutation {
        stage: &'static str,
        field: &'static str,
    },

    #[error("invalid body returned by coprocessor {stage} stage, expected {expected}: {reason}")]
    InvalidStageBody {
        stage: &'static str,
        expected: &'static str,
        reason: String,
    },
}

impl CoprocessorError {
    pub fn status_code(&self) -> ntex::http::StatusCode {
        match self {
            // TODO: triple-think the status codes
            Self::UnsupportedProtocol(_)
            | Self::InvalidUnixRequestPath(_)
            | Self::EndpointParseFailure(_, _)
            | Self::ConditionCompile(_)
            | Self::ConditionEvaluation(_)
            | Self::ForbiddenStageMutation { .. }
            | Self::InvalidStageBody { .. } => ntex::http::StatusCode::INTERNAL_SERVER_ERROR,
            _ => ntex::http::StatusCode::BAD_GATEWAY,
        }
    }
}
