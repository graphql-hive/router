use http::{
    header::{ACCEPT, CONTENT_TYPE},
    HeaderValue,
};
use lazy_static::lazy_static;
use ntex::web::HttpRequest;
use tracing::{trace, warn};

use crate::pipeline::error::PipelineErrorVariant;

lazy_static! {
    pub static ref APPLICATION_JSON_STR: &'static str = "application/json";
    pub static ref APPLICATION_JSON: HeaderValue = HeaderValue::from_static(&APPLICATION_JSON_STR);
    pub static ref APPLICATION_GRAPHQL_RESPONSE_JSON_STR: &'static str =
    "application/graphql-response+json";
    pub static ref APPLICATION_GRAPHQL_RESPONSE_JSON: HeaderValue =
    HeaderValue::from_static(&APPLICATION_GRAPHQL_RESPONSE_JSON_STR);
    pub static ref TEXT_EVENT_STREAM: &'static str = "text/event-stream";
    pub static ref MULTIPART_MIXED: &'static str = "multipart/mixed";
    /// Non-GraphQL content type, used to detect if the client can accept GraphiQL responses.
    pub static ref TEXT_HTML_CONTENT_TYPE: &'static str = "text/html";
}

/// Non-streamable (single) content types for GraphQL responses.
#[derive(PartialEq)]
pub enum SingleContentType {
    /// GraphQL over HTTP spec (`application/graphql-response+json`)
    ///
    /// Default for regular queries and mutations.
    ///
    /// Read more: https://graphql.github.io/graphql-over-http
    GraphQLResponseJSON,
    /// Legacy GraphQL over HTTP (`application/json`)
    JSON,
}

impl SingleContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SingleContentType::GraphQLResponseJSON => &APPLICATION_GRAPHQL_RESPONSE_JSON_STR,
            SingleContentType::JSON => &APPLICATION_JSON_STR,
        }
    }
    #[inline]
    pub fn default() -> Self {
        SingleContentType::GraphQLResponseJSON
    }
}

/// Streamable content types for GraphQL responses.
pub enum StreamContentType {
    /// Incremental Delivery over HTTP (`multipart/mixed`)
    ///
    /// Default for subscriptions.
    ///
    /// Read more: https://github.com/graphql/graphql-over-http/blob/c144dbd89cbea6bde0045205e34e01002f9f9ba0/rfcs/IncrementalDelivery.md
    IncrementalDelivery,
    /// GraphQL over SSE (`text/event-stream`)
    ///
    /// Only "distinct connection mode" at the moment.
    ///
    /// Read more: https://github.com/graphql/graphql-over-http/blob/d285c9f31897ea51e231ebfe8dcb481a354431c9/rfcs/GraphQLOverSSE.md
    SSE,
    /// Apollo Multipart HTTP protocol (`multipart/mixed;subscriptionSpec="1.0"`)
    ///
    /// Read more: https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol
    ApolloMultipartHTTP,
}

impl StreamContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            StreamContentType::IncrementalDelivery => &"multipart/mixed; boundary=-",
            StreamContentType::SSE => &TEXT_EVENT_STREAM,
            StreamContentType::ApolloMultipartHTTP => &r#"multipart/mixed; boundary=graphql"#,
        }
    }
    #[inline]
    pub fn default() -> Self {
        StreamContentType::IncrementalDelivery
    }
}

enum SupportedContentType {
    Single(SingleContentType),
    Stream(StreamContentType),
}

impl SupportedContentType {
    fn parse(content_type: &str) -> Option<SupportedContentType> {
        if content_type == *APPLICATION_GRAPHQL_RESPONSE_JSON_STR {
            return Some(SupportedContentType::Single(
                SingleContentType::GraphQLResponseJSON,
            ));
        }
        if content_type == *APPLICATION_JSON_STR {
            return Some(SupportedContentType::Single(SingleContentType::JSON));
        }
        if content_type.contains(*MULTIPART_MIXED)
            && content_type.contains(r#"subscriptionSpec="1.0""#)
        {
            return Some(SupportedContentType::Stream(
                StreamContentType::ApolloMultipartHTTP,
            ));
        }
        if content_type == *TEXT_EVENT_STREAM {
            return Some(SupportedContentType::Stream(StreamContentType::SSE));
        }
        if content_type == *MULTIPART_MIXED {
            return Some(SupportedContentType::Stream(
                StreamContentType::IncrementalDelivery,
            ));
        }
        None
    }
    /// Reads the header and returns a tuple of accepted/parsed content types.
    ///
    /// Returns `(SingleContentType, StreamContentType)` where:
    /// - First element: The preferred non-streamable content type (for queries/mutations)
    /// - Second element: The preferred streamable content type (for subscriptions/streaming)
    ///
    /// The content type is selected in order of appearance in the `Accept` header. For example,
    /// if the header is `Accept: text/event-stream, application/json`, the non-streamable will be
    /// `JSON` and streamable will be `SSE`.
    ///
    /// If the `Accept` header is missing or empty, defaults are used.
    fn parse_header(content_types: &str) -> (Option<SingleContentType>, Option<StreamContentType>) {
        if content_types.is_empty() {
            return (
                Some(SingleContentType::default()),
                Some(StreamContentType::default()),
            );
        }

        let mut single_content_type = None;
        let mut stream_content_type = None;

        // we must split "," to avoid false positives like when checking `(multipart/mixed, spec="1.0")`
        // with: `accept: multipart/mixed, text/event-stream;spec="1.0"`
        for content_type in content_types.split(',').map(|part| part.trim()) {
            if content_type == "*/*" {
                // wildcard means we accept everything, so we can default to our preferred types
                return (
                    Some(SingleContentType::default()),
                    Some(StreamContentType::default()),
                );
            }

            let supported_content_type = match SupportedContentType::parse(content_type) {
                Some(sct) => sct,
                None => continue,
            };
            match supported_content_type {
                SupportedContentType::Single(single) => {
                    single_content_type = single_content_type.or(Some(single))
                }
                SupportedContentType::Stream(stream) => {
                    stream_content_type = stream_content_type.or(Some(stream))
                }
            }

            if single_content_type.is_some() && stream_content_type.is_some() {
                break; // we found both, we're safe to break
            }
        }

        (single_content_type, stream_content_type)
    }
}

pub trait RequestAccepts {
    /// Whether the request can accept HTML responses. Used to determine if GraphiQL
    /// should be served.
    ///
    /// This function will never return `true` if the `Accept` header is empty or if
    /// it contains `*/*`. This is because this is not a HTML server, it's a GraphQL server.
    fn can_accept_http(&self) -> bool;
    /// Reads the request's `Accept` header and returns a tuple of accepted content types.
    ///
    /// Returns an error if no valid content types are found in the Accept header.
    fn accepted_content_type(
        &self,
    ) -> Result<(Option<SingleContentType>, Option<StreamContentType>), PipelineErrorVariant>;
}

impl RequestAccepts for HttpRequest {
    #[inline]
    fn can_accept_http(&self) -> bool {
        self.headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .map(|s| s.contains(*TEXT_HTML_CONTENT_TYPE))
            .unwrap_or(false)
    }

    #[inline]
    fn accepted_content_type(
        &self,
    ) -> Result<(Option<SingleContentType>, Option<StreamContentType>), PipelineErrorVariant> {
        let content_types = match self
            .headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
        {
            Some(t) => t,
            // None or empty Accept header means we should use defaults
            _ => {
                return Ok((
                    Some(SingleContentType::GraphQLResponseJSON),
                    Some(StreamContentType::IncrementalDelivery),
                ))
            }
        };

        let (single_content_type, stream_content_type) =
            SupportedContentType::parse_header(content_types);

        // at this point we treat no content type as "user explicitly does not support any known types"
        // this is because only empty accept header or */* is treated as "accept everything" and we check
        // that above
        match (single_content_type, stream_content_type) {
            (Some(single), Some(stream)) => Ok((Some(single), Some(stream))),
            (Some(single), None) => Ok((Some(single), None)),
            (None, Some(stream)) => Ok((None, Some(stream))),
            (None, None) => Err(PipelineErrorVariant::UnsupportedContentType),
        }
    }
}

pub trait AssertRequestJson {
    fn assert_json_content_type(&self) -> Result<(), PipelineErrorVariant>;
}

impl AssertRequestJson for HttpRequest {
    #[inline]
    fn assert_json_content_type(&self) -> Result<(), PipelineErrorVariant> {
        match self.headers().get(CONTENT_TYPE) {
            Some(value) => {
                let content_type_str = value
                    .to_str()
                    .map_err(|_| PipelineErrorVariant::InvalidHeaderValue(CONTENT_TYPE))?;
                if !content_type_str.contains(*APPLICATION_JSON_STR) {
                    warn!(
                        "Invalid content type on a POST request: {}",
                        content_type_str
                    );
                    return Err(PipelineErrorVariant::UnsupportedContentType);
                }
                Ok(())
            }
            None => {
                trace!("POST without content type detected");
                Err(PipelineErrorVariant::MissingContentTypeHeader)
            }
        }
    }
}

// TODO: tests
