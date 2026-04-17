use http::{uri::InvalidUri, StatusCode};
use rustls::server::VerifierBuilderError;
use strum::IntoStaticStr;

#[derive(thiserror::Error, Debug, IntoStaticStr)]
pub enum SubgraphExecutorError {
    #[error("Failed to parse endpoint \"{0}\" as URI: {1}")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_PARSE_FAILURE")]
    EndpointParseFailure(String, InvalidUri),
    #[error("Failed to build WebSocket endpoint \"{0}\" as URI: {1}")]
    #[strum(serialize = "SUBGRAPH_WEBSOCKET_ENDPOINT_BUILD_FAILURE")]
    WebSocketEndpointBuildFailure(String, http::Error),
    #[error("Failed to compile VRL expression for subgraph '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {1}")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_EXPRESSION_BUILD_FAILURE")]
    EndpointExpressionBuild(String, String),
    #[error("Failed to resolve VRL expression. Runtime error: {0}")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_EXPRESSION_RESOLUTION_FAILURE")]
    EndpointExpressionResolutionFailure(String),
    #[error("VRL expression resolved to a non-string value.")]
    #[strum(serialize = "SUBGRAPH_ENDPOINT_EXPRESSION_WRONG_TYPE")]
    EndpointExpressionWrongType,
    #[error(
        "Static endpoint not found for subgraph. This is an internal error and should not happen."
    )]
    #[strum(serialize = "SUBGRAPH_STATIC_ENDPOINT_NOT_FOUND")]
    StaticEndpointNotFound,
    #[error("Failed to build request to subgraph: {0}")]
    #[strum(serialize = "SUBGRAPH_REQUEST_BUILD_FAILURE")]
    RequestBuildFailure(#[from] http::Error),
    #[error("Failed to send request to subgraph: {0}")]
    #[strum(serialize = "SUBGRAPH_REQUEST_FAILURE")]
    RequestFailure(#[from] hyper_util::client::legacy::Error),
    #[error("Failed to receive response: {0}")]
    #[strum(serialize = "SUBGRAPH_RESPONSE_FAILURE")]
    ResponseFailure(#[from] hyper::Error),
    #[error("Failed to serialize variable \"{0}\": {1}")]
    #[strum(serialize = "SUBGRAPH_VARIABLES_SERIALIZATION_FAILURE")]
    VariablesSerializationFailure(String, sonic_rs::Error),
    #[error("Failed to compile VRL expression for timeout for subgraph '{0}'. Please check your VRL expression for syntax errors. Diagnostic: {0}")]
    #[strum(serialize = "SUBGRAPH_TIMEOUT_EXPRESSION_BUILD_FAILURE")]
    RequestTimeoutExpressionBuild(String, String),
    #[error("Failed to resolve VRL expression for timeout for subgraph. Runtime error: {0}")]
    #[strum(serialize = "SUBGRAPH_TIMEOUT_EXPRESSION_RESOLUTION_FAILURE")]
    TimeoutExpressionResolution(String),
    #[error("Request to subgraph timed out")]
    #[strum(serialize = "SUBGRAPH_REQUEST_TIMEOUT")]
    RequestTimeout(#[from] tokio::time::error::Elapsed),
    #[error("Failed to read response body from subgraph \"{0}\": {1}")]
    #[strum(serialize = "SUBGRAPH_RESPONSE_BODY_READ_FAILURE")]
    ResponseBodyReadFailure(String, String),
    #[error("Received empty response body from subgraph \"{0}\"")]
    #[strum(serialize = "SUBGRAPH_RESPONSE_BODY_EMPTY")]
    EmptyResponseBody(String),
    #[error("Failed to deserialize subgraph response: {0}")]
    #[strum(serialize = "SUBGRAPH_RESPONSE_DESERIALIZATION_FAILURE")]
    ResponseDeserializationFailure(sonic_rs::Error),
    #[error(transparent)]
    #[strum(serialize = "SUBGRAPH_HTTPS_CERTS_FAILURE")]
    TlsCertificatesError(#[from] TlsCertificatesError),
    #[error("Unsupported content-type '{0}': expected 'multipart/mixed' or 'text/event-stream' for HTTP subscriptions")]
    #[strum(serialize = "SUBGRAPH_SUBSCRIPTION_UNSUPPORTED_CONTENT_TYPE")]
    UnsupportedContentTypeError(String),
    #[error("Failed to connect WebSocket to '{0}': {1}")]
    #[strum(serialize = "SUBGRAPH_WEBSOCKET_CONNECT_FAILURE")]
    WebSocketConnectFailure(String, String),
    #[error("WebSocket protocol handshake with '{0}' failed: {1}")]
    #[strum(serialize = "SUBGRAPH_WEBSOCKET_HANDSHAKE_FAILURE")]
    WebSocketHandshakeFailure(String, String),
    #[error("WebSocket subscription stream at '{0}' closed before sending any response")]
    #[strum(serialize = "SUBGRAPH_WEBSOCKET_STREAM_CLOSED_EMPTY")]
    WebSocketStreamClosedEmpty(String),
    #[error("WebSocket executor arbiter channel closed unexpectedly")]
    #[strum(serialize = "SUBGRAPH_WEBSOCKET_ARBITER_CHANNEL_CLOSED")]
    WebSocketArbiterChannelClosed,
    #[error("Failed to parse multipart boundary from Content-Type header: {0}")]
    #[strum(serialize = "SUBGRAPH_SUBSCRIPTION_MULTIPART_BOUNDARY_PARSE_FAILURE")]
    MultipartBoundaryParseFailure(String),
    #[error("Error reading multipart subscription stream: {0}")]
    #[strum(serialize = "SUBGRAPH_SUBSCRIPTION_MULTIPART_STREAM_ERROR")]
    MultipartStreamError(String),
    #[error("Error reading SSE subscription stream: {0}")]
    #[strum(serialize = "SUBGRAPH_SUBSCRIPTION_SSE_STREAM_ERROR")]
    SseStreamError(String),
    #[error("Subgraph stream responded with a not-OK status code '{0}'")]
    #[strum(serialize = "SUBGRAPH_STREAM_STATUS_CODE_NOT_OK")]
    StreamStatusCodeNotOk(StatusCode),
    #[error("Subgraph HTTP callback responded with a not-OK status code '{0}'")]
    #[strum(serialize = "SUBGRAPH_HTTP_CALLBACK_STATUS_CODE_NOT_OK")]
    HttpCallbackStatusCodeNotOk(StatusCode),
    #[error("HTTP Callback protocol does not support single-shot execution, use it only for subscriptions")]
    #[strum(serialize = "SUBGRAPH_HTTP_CALLBACK_NO_SINGLE")]
    HttpCallbackNoSingle,
    #[error("HTTP Callback protocol configured for subgraph but no callback configuration provided for router")]
    #[strum(serialize = "SUBGRAPH_HTTP_CALLBACK_NOT_CONFIGURED")]
    HttpCallbackNotConfigured,
}

impl SubgraphExecutorError {
    pub fn error_code(&self) -> &'static str {
        self.into()
    }
}

#[derive(thiserror::Error, Debug, IntoStaticStr)]
pub enum TlsCertificatesError {
    #[error("Failed to initialize or load native TLS root certificates: {0}")]
    NativeTlsCertificatesError(std::io::Error),
    #[error("Failed to load custom TLS certificate {0}: {1}")]
    CustomTlsCertificatesError(&'static str, rustls::pki_types::pem::Error),
    #[error("Unexpected invalid certificates: {0}")]
    InvalidTlsCertificates(String),
    #[error("Failed to build TLS client configuration: {0}")]
    TlsConfigFailure(#[from] rustls::Error),
    #[error("Failed to build TLS verifier: {0}")]
    TlsVerifierFailure(#[from] VerifierBuilderError),
}
