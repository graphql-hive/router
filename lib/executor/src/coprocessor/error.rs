use hive_router_config::coprocessor::CoprocessorProtocol;
use hive_router_internal::expressions::ExpressionCompileError;
use http::header::ToStrError;
use http::uri::InvalidUri;
use ntex::http::error::PayloadError;
use strum::IntoStaticStr;

#[derive(thiserror::Error, Debug, IntoStaticStr)]
pub enum CoprocessorError {
    #[error("coprocessor protocol '{0:?}' is not supported")]
    #[strum(serialize = "COPROCESSOR_UNSUPPORTED_PROTOCOL")]
    UnsupportedProtocol(CoprocessorProtocol),

    #[error("coprocessor unix:// request path must start with '/', received '{0}'")]
    #[strum(serialize = "COPROCESSOR_INVALID_UNIX_REQUEST_PATH")]
    InvalidUnixRequestPath(String),

    #[error("failed to parse coprocessor endpoint URI '{0}': {1}")]
    #[strum(serialize = "COPROCESSOR_ENDPOINT_PARSE_FAILURE")]
    EndpointParseFailure(String, InvalidUri),

    #[error("failed to build coprocessor request: {0}")]
    #[strum(serialize = "COPROCESSOR_REQUEST_BUILD_FAILURE")]
    RequestBuildFailure(#[source] http::Error),

    #[error("coprocessor request execution failed: {0}")]
    #[strum(serialize = "COPROCESSOR_REQUEST_EXECUTION_FAILURE")]
    RequestExecutionFailure(#[source] hyper_util::client::legacy::Error),

    #[error("coprocessor returned non-success status: {0}")]
    #[strum(serialize = "COPROCESSOR_UNEXPECTED_STATUS")]
    UnexpectedStatus(http::StatusCode),

    #[error("coprocessor request to '{endpoint}' timed out after {timeout_ms}ms")]
    #[strum(serialize = "COPROCESSOR_REQUEST_TIMEOUT")]
    RequestTimeout { endpoint: String, timeout_ms: u128 },

    #[error("failed reading coprocessor response body: {0}")]
    #[strum(serialize = "COPROCESSOR_RESPONSE_BODY_READ_FAILURE")]
    ResponseBodyReadFailure(#[source] hyper::Error),

    #[error("invalid coprocessor content-encoding header: {0}")]
    #[strum(serialize = "COPROCESSOR_INVALID_CONTENT_ENCODING_HEADER")]
    InvalidContentEncodingHeader(#[source] ToStrError),

    #[error("unsupported stacked content-encoding from coprocessor: '{0}'")]
    #[strum(serialize = "COPROCESSOR_UNSUPPORTED_STACKED_CONTENT_ENCODING")]
    UnsupportedStackedContentEncoding(String),

    #[error("unsupported content-encoding from coprocessor: '{0}'")]
    #[strum(serialize = "COPROCESSOR_UNSUPPORTED_CONTENT_ENCODING")]
    UnsupportedContentEncoding(String),

    #[error("failed to decompress coprocessor response using '{encoding}': {source}")]
    #[strum(serialize = "COPROCESSOR_RESPONSE_DECOMPRESSION_FAILURE")]
    ResponseDecompressionFailure {
        encoding: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to compile coprocessor condition expression: {0}")]
    #[strum(serialize = "COPROCESSOR_CONDITION_COMPILE_ERROR")]
    ConditionCompile(#[from] ExpressionCompileError),

    #[error("failed to evaluate coprocessor condition: {0}")]
    #[strum(serialize = "COPROCESSOR_CONDITION_EVALUATION_ERROR")]
    ConditionEvaluation(String),

    #[error("failed to read router request body for coprocessor: {0}")]
    #[strum(serialize = "COPROCESSOR_REQUEST_BODY_READ_ERROR")]
    RequestBodyRead(#[from] PayloadError),

    #[error("invalid UTF-8 body bytes in {context}: {source}")]
    #[strum(serialize = "COPROCESSOR_INVALID_UTF8_BODY_ERROR")]
    InvalidUtf8Body {
        context: &'static str,
        #[source]
        source: std::str::Utf8Error,
    },

    #[error("failed to deserialize coprocessor response payload: {0}")]
    #[strum(serialize = "COPROCESSOR_RESPONSE_DESERIALIZE_ERROR")]
    ResponseDeserialize(#[from] sonic_rs::Error),

    #[error("coprocessor returned unsupported version {0}")]
    #[strum(serialize = "COPROCESSOR_UNSUPPORTED_VERSION_ERROR")]
    UnsupportedVersion(u8),

    #[error("invalid HTTP header name in coprocessor payload: {0}")]
    #[strum(serialize = "COPROCESSOR_INVALID_HEADER_NAME_ERROR")]
    InvalidHeaderName(String),

    #[error("invalid HTTP header value in coprocessor payload: {0}")]
    #[strum(serialize = "COPROCESSOR_INVALID_HEADER_VALUE_ERROR")]
    InvalidHeaderValue(String),

    #[error("invalid HTTP method in coprocessor payload: {0}")]
    #[strum(serialize = "COPROCESSOR_INVALID_METHOD_ERROR")]
    InvalidMethod(String),

    #[error("invalid request path in coprocessor payload: {0}")]
    #[strum(serialize = "COPROCESSOR_INVALID_PATH_ERROR")]
    InvalidPath(String),

    #[error("coprocessor {stage} stage cannot mutate '{field}'")]
    #[strum(serialize = "COPROCESSOR_FORBIDDEN_STAGE_MUTATION_ERROR")]
    ForbiddenStageMutation {
        stage: &'static str,
        field: &'static str,
    },

    #[error("invalid body returned by coprocessor {stage} stage, expected {expected}: {reason}")]
    #[strum(serialize = "COPROCESSOR_INVALID_STAGE_BODY_ERROR")]
    InvalidStageBody {
        stage: &'static str,
        expected: &'static str,
        reason: String,
    },
}

impl CoprocessorError {
    pub fn status_code(&self) -> ntex::http::StatusCode {
        // Let's use the same status code for all errors.
        // This way we won't leak anything to the client.
        ntex::http::StatusCode::INTERNAL_SERVER_ERROR
    }
    pub fn error_code(&self) -> &'static str {
        self.into()
    }
}
