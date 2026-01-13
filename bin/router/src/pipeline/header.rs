use headers_accept::Accept;
use headers_core::Error as HeaderError;
use http::header::ACCEPT;
use mediatype::MediaType;
use ntex::web::HttpRequest;
use std::str::FromStr;
use tracing::error;

use crate::pipeline::error::PipelineError;

/// Non-GraphQL content type, used to detect if the client can accept GraphiQL responses.
pub const TEXT_HTML_MIME: &str = "text/html";

const JSON_MEDIA_TYPE: MediaType =
    MediaType::new(mediatype::names::APPLICATION, mediatype::names::JSON);

const GRAPHQL_RESPONSE_JSON_MEDIA_TYPE: MediaType = MediaType::from_parts(
    mediatype::names::APPLICATION,
    mediatype::Name::new_unchecked("graphql-response"),
    Some(mediatype::names::JSON),
    &[],
);

const INCREMENTAL_DELIVERY_MEDIA_TYPE: MediaType =
    MediaType::new(mediatype::names::MULTIPART, mediatype::names::MIXED);

const SSE_MEDIA_TYPE: MediaType =
    MediaType::new(mediatype::names::TEXT, mediatype::names::EVENT_STREAM);

const APOLLO_MULTIPART_HTTP_MEDIA_TYPE: MediaType = MediaType::from_parts(
    mediatype::names::MULTIPART,
    mediatype::names::MIXED,
    None,
    &[(
        mediatype::Name::new_unchecked("subscriptionSpec"),
        mediatype::Value::new_unchecked("1.0"),
    )],
);

const SUPPORTED_SINGLE_MEDIA_TYPES: &[MediaType] =
    &[JSON_MEDIA_TYPE, GRAPHQL_RESPONSE_JSON_MEDIA_TYPE];

const SUPPORTED_STREAM_MEDIA_TYPES: &[MediaType] = &[
    SSE_MEDIA_TYPE,
    INCREMENTAL_DELIVERY_MEDIA_TYPE,
    APOLLO_MULTIPART_HTTP_MEDIA_TYPE,
];

/// Non-streamable (single) content types for GraphQL responses.
#[derive(PartialEq, Default, Debug, Clone)]
pub enum SingleContentType {
    /// GraphQL over HTTP spec (`application/graphql-response+json`)
    ///
    /// Read more: https://graphql.github.io/graphql-over-http
    GraphQLResponseJSON,
    /// Legacy GraphQL over HTTP (`application/json`)
    ///
    /// Default for regular queries and mutations.
    ///
    /// Read more: https://graphql.github.io/graphql-over-http
    #[default]
    JSON,
}

impl SingleContentType {
    pub fn from_media_type(media_type: Option<&MediaType>) -> Option<SingleContentType> {
        let media_type = media_type?;
        if media_type == &GRAPHQL_RESPONSE_JSON_MEDIA_TYPE {
            Some(SingleContentType::GraphQLResponseJSON)
        } else if media_type == &JSON_MEDIA_TYPE {
            Some(SingleContentType::JSON)
        } else {
            None
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            SingleContentType::GraphQLResponseJSON => "application/graphql-response+json",
            SingleContentType::JSON => "application/json",
        }
    }
}

/// Streamable content types for GraphQL responses.
#[derive(PartialEq, Default, Debug)]
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
    pub fn from_media_type(media_type: Option<&MediaType>) -> Option<StreamContentType> {
        let media_type = media_type?;
        if media_type == &INCREMENTAL_DELIVERY_MEDIA_TYPE {
            Some(StreamContentType::IncrementalDelivery)
        } else if media_type == &SSE_MEDIA_TYPE {
            Some(StreamContentType::SSE)
        } else if media_type == &APOLLO_MULTIPART_HTTP_MEDIA_TYPE {
            Some(StreamContentType::ApolloMultipartHTTP)
        } else {
            None
        }
    }

    /// Will return the string representation of the content type as to be sent
    /// back to the client in the `Content-Type` header.
    ///
    /// For example, if during negotiation the client accepts `multipart/mixed;subscriptionsSpec="1.0"`,
    /// this function will return `multipart/mixed; boundary=graphql` because that is the expected
    /// content type value for multipart HTTP responses.
    pub const fn as_str(&self) -> &'static str {
        match self {
            StreamContentType::IncrementalDelivery => "multipart/mixed; boundary=-",
            StreamContentType::SSE => "text/event-stream",
            StreamContentType::ApolloMultipartHTTP => r#"multipart/mixed; boundary=graphql"#,
        }
    }
}

/// Reads the `Accept` header contents and returns a tuple of accepted/parsed content types.
/// It perform negotiation and respects q-weights.
fn negotiate_content_type(
    accept_header: &str,
) -> Result<(Option<SingleContentType>, Option<StreamContentType>), HeaderError> {
    if accept_header.is_empty() {
        return Ok((
            Some(SingleContentType::default()),
            Some(StreamContentType::default()),
        ));
    }
    let accept = Accept::from_str(accept_header)?;
    let agreed_single =
        SingleContentType::from_media_type(accept.negotiate(SUPPORTED_SINGLE_MEDIA_TYPES));
    let agreed_stream =
        StreamContentType::from_media_type(accept.negotiate(SUPPORTED_STREAM_MEDIA_TYPES));
    Ok((agreed_single, agreed_stream))
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
    fn negotiate(
        &self,
    ) -> Result<(Option<SingleContentType>, Option<StreamContentType>), PipelineError>;
}

impl RequestAccepts for HttpRequest {
    #[inline]
    fn can_accept_http(&self) -> bool {
        self.headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .map(|s| s.contains(TEXT_HTML_MIME))
            .unwrap_or(false)
    }

    #[inline]
    fn negotiate(
        &self,
    ) -> Result<(Option<SingleContentType>, Option<StreamContentType>), PipelineError> {
        let content_types = self
            .headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");

        let agreed = negotiate_content_type(content_types).map_err(|err| {
            error!("Failed to parse Accept header: {}", err);
            PipelineError::InvalidHeaderValue(ACCEPT)
        })?;

        // at this point we treat no content type as "user explicitly does not support any known types"
        // this is because only empty accept header or */* is treated as "accept everything" and we check
        // that above
        if agreed.0.is_none() && agreed.1.is_none() {
            Err(PipelineError::UnsupportedContentType)
        } else {
            Ok(agreed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiate_single_and_stream_content_types() {
        let cases = vec![
            (
                "",
                Some(SingleContentType::JSON),
                Some(StreamContentType::IncrementalDelivery),
            ),
            (
                r#"application/json, text/event-stream, multipart/mixed;subscriptionSpec="1.0""#,
                Some(SingleContentType::JSON),
                Some(StreamContentType::ApolloMultipartHTTP),
            ),
            (
                r#"application/graphql-response+json, multipart/mixed;q=0.5, text/event-stream;q=1"#,
                Some(SingleContentType::GraphQLResponseJSON),
                Some(StreamContentType::SSE),
            ),
            (
                r#"application/json;q=0.5, application/graphql-response+json;q=1"#,
                Some(SingleContentType::GraphQLResponseJSON),
                None,
            ),
            (
                r#"text/event-stream;q=0.5, multipart/mixed;q=1;subscriptionSpec="1.0""#,
                None,
                Some(StreamContentType::ApolloMultipartHTTP),
            ),
            (
                r#"text/event-stream, application/json"#,
                Some(SingleContentType::JSON),
                Some(StreamContentType::SSE),
            ),
        ];

        for (accept_header, expected_single, expected_stream) in cases {
            let (single, stream) =
                negotiate_content_type(accept_header).expect("unable to parse accept header");
            assert_eq!(single, expected_single, "wrong single content type");
            assert_eq!(stream, expected_stream, "wrong stream content type");
        }
    }
}
