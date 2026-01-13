use http::{header::ACCEPT, HeaderValue};
use lazy_static::lazy_static;
use ntex::web::HttpRequest;

use crate::pipeline::error::PipelineError;

pub const APPLICATION_JSON_STR: &str = "application/json";
const APPLICATION_GRAPHQL_RESPONSE_JSON_STR: &str = "application/graphql-response+json";
const TEXT_EVENT_STREAM: &str = "text/event-stream";
const MULTIPART_MIXED: &str = "multipart/mixed";
/// Non-GraphQL content type, used to detect if the client can accept GraphiQL responses.
pub const TEXT_HTML_CONTENT_TYPE: &str = "text/html";

lazy_static! {
    pub static ref APPLICATION_JSON: HeaderValue = HeaderValue::from_static(APPLICATION_JSON_STR);
    pub static ref APPLICATION_GRAPHQL_RESPONSE_JSON: HeaderValue =
        HeaderValue::from_static(APPLICATION_GRAPHQL_RESPONSE_JSON_STR);
}

/// Non-streamable (single) content types for GraphQL responses.
#[derive(PartialEq, Default, Debug, Clone, Copy)]
pub enum SingleContentType {
    /// GraphQL over HTTP spec (`application/graphql-response+json`)
    ///
    /// Default for regular queries and mutations.
    ///
    /// Read more: https://graphql.github.io/graphql-over-http
    GraphQLResponseJSON,
    /// Legacy GraphQL over HTTP (`application/json`)
    #[default]
    JSON,
}

impl SingleContentType {
    pub const fn as_str(&self) -> &'static str {
        match self {
            SingleContentType::GraphQLResponseJSON => APPLICATION_GRAPHQL_RESPONSE_JSON_STR,
            SingleContentType::JSON => APPLICATION_JSON_STR,
        }
    }
}

/// Streamable content types for GraphQL responses.
#[derive(PartialEq, Default, Debug, Clone, Copy)]
pub enum StreamContentType {
    /// Incremental Delivery over HTTP (`multipart/mixed`)
    ///
    /// Default for subscriptions.
    ///
    /// Read more: https://github.com/graphql/graphql-over-http/blob/c144dbd89cbea6bde0045205e34e01002f9f9ba0/rfcs/IncrementalDelivery.md
    #[default]
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
    pub const fn as_str(&self) -> &'static str {
        match self {
            StreamContentType::IncrementalDelivery => "multipart/mixed; boundary=-",
            StreamContentType::SSE => TEXT_EVENT_STREAM,
            StreamContentType::ApolloMultipartHTTP => r#"multipart/mixed; boundary=graphql"#,
        }
    }
}

#[derive(Debug)]
enum SupportedContentType {
    Single(SingleContentType),
    Stream(StreamContentType),
}

impl SupportedContentType {
    fn parse(content_type: &str) -> Option<SupportedContentType> {
        if content_type == APPLICATION_GRAPHQL_RESPONSE_JSON_STR {
            return Some(SupportedContentType::Single(
                SingleContentType::GraphQLResponseJSON,
            ));
        }
        if content_type == APPLICATION_JSON_STR {
            return Some(SupportedContentType::Single(SingleContentType::JSON));
        }
        if content_type.contains(MULTIPART_MIXED)
            && content_type.contains(r#"subscriptionSpec="1.0""#)
        {
            return Some(SupportedContentType::Stream(
                StreamContentType::ApolloMultipartHTTP,
            ));
        }
        if content_type == TEXT_EVENT_STREAM {
            return Some(SupportedContentType::Stream(StreamContentType::SSE));
        }
        if content_type == MULTIPART_MIXED {
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
    ) -> Result<(Option<SingleContentType>, Option<StreamContentType>), PipelineError>;
}

impl RequestAccepts for HttpRequest {
    #[inline]
    fn can_accept_http(&self) -> bool {
        self.headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .map(|s| s.contains(TEXT_HTML_CONTENT_TYPE))
            .unwrap_or(false)
    }

    #[inline]
    fn accepted_content_type(
        &self,
    ) -> Result<(Option<SingleContentType>, Option<StreamContentType>), PipelineError> {
        let content_types = self
            .headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");

        let parsed @ (single, stream) = SupportedContentType::parse_header(content_types);

        // at this point we treat no content type as "user explicitly does not support any known types"
        // this is because only empty accept header or */* is treated as "accept everything" and we check
        // that above
        if single.is_none() && stream.is_none() {
            Err(PipelineError::UnsupportedContentType)
        } else {
            Ok(parsed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod supported_content_type_parse {
        use super::*;

        #[test]
        fn test_application_graphql_response_json() {
            let result = SupportedContentType::parse("application/graphql-response+json");
            assert!(matches!(
                result,
                Some(SupportedContentType::Single(
                    SingleContentType::GraphQLResponseJSON
                ))
            ));
        }

        #[test]
        fn test_application_json() {
            let result = SupportedContentType::parse("application/json");
            assert!(matches!(
                result,
                Some(SupportedContentType::Single(SingleContentType::JSON))
            ));
        }

        #[test]
        fn test_text_event_stream() {
            let result = SupportedContentType::parse("text/event-stream");
            assert!(matches!(
                result,
                Some(SupportedContentType::Stream(StreamContentType::SSE))
            ));
        }

        #[test]
        fn test_multipart_mixed() {
            let result = SupportedContentType::parse("multipart/mixed");
            assert!(matches!(
                result,
                Some(SupportedContentType::Stream(
                    StreamContentType::IncrementalDelivery
                ))
            ));
        }

        #[test]
        fn test_apollo_multipart_http() {
            let result = SupportedContentType::parse(r#"multipart/mixed; subscriptionSpec="1.0""#);
            assert!(matches!(
                result,
                Some(SupportedContentType::Stream(
                    StreamContentType::ApolloMultipartHTTP
                ))
            ));
        }

        #[test]
        fn test_apollo_multipart_http_reversed_params() {
            let result = SupportedContentType::parse(r#"subscriptionSpec="1.0"; multipart/mixed"#);
            assert!(matches!(
                result,
                Some(SupportedContentType::Stream(
                    StreamContentType::ApolloMultipartHTTP
                ))
            ));
        }

        #[test]
        fn test_unsupported_content_type() {
            let result = SupportedContentType::parse("text/html");
            assert!(result.is_none());
        }

        #[test]
        fn test_empty_string() {
            let result = SupportedContentType::parse("");
            assert!(result.is_none());
        }
    }

    mod supported_content_type_parse_header {
        use super::*;

        #[test]
        fn test_empty_header_returns_defaults() {
            let (single, stream) = SupportedContentType::parse_header("");
            assert!(matches!(single, Some(SingleContentType::JSON)));
            assert!(matches!(
                stream,
                Some(StreamContentType::IncrementalDelivery)
            ));
        }

        #[test]
        fn test_wildcard_returns_defaults() {
            let (single, stream) = SupportedContentType::parse_header("*/*");
            assert!(matches!(single, Some(SingleContentType::JSON)));
            assert!(matches!(
                stream,
                Some(StreamContentType::IncrementalDelivery)
            ));
        }

        #[test]
        fn test_single_content_type_only() {
            let (single, stream) = SupportedContentType::parse_header("application/json");
            assert!(matches!(single, Some(SingleContentType::JSON)));
            assert!(stream.is_none());
        }

        #[test]
        fn test_stream_content_type_only() {
            let (single, stream) = SupportedContentType::parse_header("text/event-stream");
            assert!(single.is_none());
            assert!(matches!(stream, Some(StreamContentType::SSE)));
        }

        #[test]
        fn test_multiple_content_types_order_matters() {
            // First single type should be selected
            let (single, stream) = SupportedContentType::parse_header(
                "application/json, application/graphql-response+json",
            );
            assert!(matches!(single, Some(SingleContentType::JSON)));
            assert!(stream.is_none());

            // Reversed order
            let (single, stream) = SupportedContentType::parse_header(
                "application/graphql-response+json, application/json",
            );
            assert!(matches!(
                single,
                Some(SingleContentType::GraphQLResponseJSON)
            ));
            assert!(stream.is_none());
        }

        #[test]
        fn test_mixed_single_and_stream_types() {
            let (single, stream) =
                SupportedContentType::parse_header("text/event-stream, application/json");
            assert!(matches!(single, Some(SingleContentType::JSON)));
            assert!(matches!(stream, Some(StreamContentType::SSE)));
        }

        #[test]
        fn test_order_of_appearance_respected() {
            // SSE before multipart/mixed
            let (_single, stream) =
                SupportedContentType::parse_header("text/event-stream, multipart/mixed");
            assert!(matches!(stream, Some(StreamContentType::SSE)));

            // multipart/mixed before SSE
            let (_single, stream) =
                SupportedContentType::parse_header("multipart/mixed, text/event-stream");
            assert!(matches!(
                stream,
                Some(StreamContentType::IncrementalDelivery)
            ));
        }

        #[test]
        fn test_apollo_multipart_with_comma_in_accept_header() {
            // This tests the split logic to avoid false positives
            let (_single, stream) = SupportedContentType::parse_header(
                r#"multipart/mixed;subscriptionSpec="1.0", text/event-stream"#,
            );
            assert!(matches!(
                stream,
                Some(StreamContentType::ApolloMultipartHTTP)
            ));
        }

        #[test]
        fn test_whitespace_handling() {
            let (single, stream) = SupportedContentType::parse_header(
                "application/json , text/event-stream , multipart/mixed",
            );
            assert!(matches!(single, Some(SingleContentType::JSON)));
            assert!(matches!(stream, Some(StreamContentType::SSE)));
        }

        #[test]
        fn test_unsupported_types_ignored() {
            let (single, stream) =
                SupportedContentType::parse_header("text/html, application/json, text/plain");
            assert!(matches!(single, Some(SingleContentType::JSON)));
            assert!(stream.is_none());
        }

        #[test]
        fn test_all_unsupported_types() {
            let (single, stream) =
                SupportedContentType::parse_header("text/html, text/plain, application/xml");
            assert!(single.is_none());
            assert!(stream.is_none());
        }

        #[test]
        fn test_stops_after_finding_both_types() {
            // Should stop processing after finding both single and stream types
            let (single, stream) = SupportedContentType::parse_header(
                "application/json, text/event-stream, application/graphql-response+json, multipart/mixed",
            );
            assert!(matches!(single, Some(SingleContentType::JSON)));
            assert!(matches!(stream, Some(StreamContentType::SSE)));
        }

        #[test]
        fn test_wildcard_with_other_types_returns_defaults() {
            // Wildcard should return defaults regardless of other types
            let (single, stream) =
                SupportedContentType::parse_header("application/json, */*, text/event-stream");
            assert!(matches!(single, Some(SingleContentType::JSON)));
            assert!(matches!(
                stream,
                Some(StreamContentType::IncrementalDelivery)
            ));
        }
    }
}
