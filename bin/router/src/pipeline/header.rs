use headers_accept::Accept;
use http::header::ACCEPT;
use mediatype::MediaType;
use ntex::web::HttpRequest;
use std::str::FromStr;
use strum::{AsRefStr, EnumString, IntoStaticStr};
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
#[derive(PartialEq, Default, Debug, Clone, IntoStaticStr, EnumString, AsRefStr)]
pub enum SingleContentType {
    /// GraphQL over HTTP spec (`application/graphql-response+json`)
    ///
    /// Read more: https://graphql.github.io/graphql-over-http
    #[strum(serialize = "application/graphql-response+json")]
    GraphQLResponseJSON,
    /// Legacy GraphQL over HTTP (`application/json`)
    ///
    /// Default for regular queries and mutations.
    ///
    /// Read more: https://graphql.github.io/graphql-over-http
    #[default]
    #[strum(serialize = "application/json")]
    JSON,
}

impl From<&MediaType<'_>> for SingleContentType {
    fn from(media_type: &MediaType) -> Self {
        if media_type == &GRAPHQL_RESPONSE_JSON_MEDIA_TYPE {
            SingleContentType::GraphQLResponseJSON
        } else {
            SingleContentType::JSON
        }
    }
}

/// Streamable content types for GraphQL responses.
///
/// The into static str will return the string representation of the content type as to be sent
/// back to the client in the `Content-Type` header.
///
/// For example, if during negotiation the client accepts `multipart/mixed;subscriptionsSpec="1.0"`,
/// this function will return `multipart/mixed; boundary=graphql` because that is the expected
/// content type value for multipart HTTP responses.
#[derive(PartialEq, Default, Debug, IntoStaticStr, EnumString, AsRefStr)]
pub enum StreamContentType {
    /// Incremental Delivery over HTTP (`multipart/mixed`)
    ///
    /// Default for subscriptions.
    ///
    /// Read more: https://github.com/graphql/graphql-over-http/blob/c144dbd89cbea6bde0045205e34e01002f9f9ba0/rfcs/IncrementalDelivery.md
    #[default]
    #[strum(serialize = "multipart/mixed; boundary=-")]
    IncrementalDelivery,
    /// GraphQL over SSE (`text/event-stream`)
    ///
    /// Only "distinct connection mode" at the moment.
    ///
    /// Read more: https://github.com/graphql/graphql-over-http/blob/d285c9f31897ea51e231ebfe8dcb481a354431c9/rfcs/GraphQLOverSSE.md
    #[strum(serialize = "text/event-stream")]
    SSE,
    /// Apollo Multipart HTTP protocol (`multipart/mixed;subscriptionSpec="1.0"`)
    ///
    /// Read more: https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol
    #[strum(serialize = r#"multipart/mixed; boundary=graphql"#)]
    ApolloMultipartHTTP,
}

impl From<&MediaType<'_>> for StreamContentType {
    fn from(media_type: &MediaType) -> Self {
        if media_type == &INCREMENTAL_DELIVERY_MEDIA_TYPE {
            StreamContentType::IncrementalDelivery
        } else if media_type == &SSE_MEDIA_TYPE {
            StreamContentType::SSE
        } else {
            StreamContentType::ApolloMultipartHTTP
        }
    }
}

/// The agreed content types after negotiation. Client may accept only single, only stream, or both,
/// it's important we convey this message because it affects how we process the response.
#[derive(PartialEq, Debug)]
pub enum ResponseMode {
    /// Can only single response, error on subscriptions.
    SingleOnly(SingleContentType),
    /// Will always respond, queries are streamed so are subscriptions, errors are also streamed.
    StreamOnly(StreamContentType),
    /// Will always respond, queries are single responses, subscriptions are streams. errors are single responses.
    Dual(SingleContentType, StreamContentType),
}

// `#[default]` attribute may only be used on unit enum variants, so we have to implement it
impl Default for ResponseMode {
    fn default() -> Self {
        ResponseMode::SingleOnly(SingleContentType::default())
    }
}

impl ResponseMode {
    pub fn can_single(&self) -> bool {
        matches!(self, ResponseMode::SingleOnly(_) | ResponseMode::Dual(_, _))
    }
    pub fn single_content_type(&self) -> Option<&SingleContentType> {
        match self {
            ResponseMode::SingleOnly(single) => Some(single),
            ResponseMode::Dual(single, _) => Some(single),
            ResponseMode::StreamOnly(_) => None,
        }
    }
    pub fn can_stream(&self) -> bool {
        matches!(self, ResponseMode::StreamOnly(_) | ResponseMode::Dual(_, _))
    }
    pub fn stream_content_type(&self) -> Option<&StreamContentType> {
        match self {
            ResponseMode::StreamOnly(stream) => Some(stream),
            ResponseMode::Dual(_, stream) => Some(stream),
            ResponseMode::SingleOnly(_) => None,
        }
    }
}

/// Reads the `Accept` header contents and returns a tuple of accepted/parsed content types.
/// It perform negotiation and respects q-weights.
fn negotiate_content_type(
    accept_header: Option<&str>,
) -> Result<Option<ResponseMode>, <Accept as FromStr>::Err> {
    let accept_header = accept_header.unwrap_or_default();
    if accept_header.is_empty() {
        return Ok(Some(ResponseMode::Dual(
            SingleContentType::default(),
            StreamContentType::default(),
        )));
    }
    let accept = Accept::from_str(accept_header)?;
    let agreed_single = accept
        .negotiate(SUPPORTED_SINGLE_MEDIA_TYPES)
        .map(|t| t.into());
    let agreed_stream = accept
        .negotiate(SUPPORTED_STREAM_MEDIA_TYPES)
        .map(|t| t.into());
    match (agreed_single, agreed_stream) {
        (Some(single), Some(stream)) => Ok(Some(ResponseMode::Dual(single, stream))),
        (Some(single), None) => Ok(Some(ResponseMode::SingleOnly(single))),
        (None, Some(stream)) => Ok(Some(ResponseMode::StreamOnly(stream))),
        (None, None) => Ok(None),
    }
}

pub trait RequestAccepts {
    /// Whether the request can accept HTML responses. Used to determine if GraphiQL
    /// should be served.
    ///
    /// This function will never return `true` if the `Accept` header is empty or if
    /// it contains `*/*`. This is because this is not a HTML server, it's a GraphQL server.
    fn can_accept_http(&self) -> bool;
    /// Reads the request's `Accept` header and returns the agreed response mode.
    ///
    /// Returns an error if no valid content types are found in the Accept header.
    fn negotiate(&self) -> Result<Option<ResponseMode>, PipelineError>;
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
    fn negotiate(&self) -> Result<Option<ResponseMode>, PipelineError> {
        let content_types = self
            .headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok());

        let agreed = negotiate_content_type(content_types).map_err(|err| {
            error!("Failed to parse Accept header: {}", err);
            PipelineError::InvalidHeaderValue(ACCEPT)
        })?;

        // at this point we treat no content type as "user explicitly does not support any known types"
        // this is because only empty accept header or */* is treated as "accept everything" and we check
        // that above
        if agreed.is_none() {
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
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::IncrementalDelivery,
                ),
            ),
            (
                r#"application/json, text/event-stream, multipart/mixed;subscriptionSpec="1.0""#,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::ApolloMultipartHTTP,
                ),
            ),
            (
                r#"application/graphql-response+json, multipart/mixed;q=0.5, text/event-stream;q=1"#,
                ResponseMode::Dual(
                    SingleContentType::GraphQLResponseJSON,
                    StreamContentType::SSE,
                ),
            ),
            (
                r#"application/json;q=0.5, application/graphql-response+json;q=1"#,
                ResponseMode::SingleOnly(SingleContentType::GraphQLResponseJSON),
            ),
            (
                r#"text/event-stream;q=0.5, multipart/mixed;q=1;subscriptionSpec="1.0""#,
                ResponseMode::StreamOnly(StreamContentType::ApolloMultipartHTTP),
            ),
            (
                r#"text/event-stream, application/json"#,
                ResponseMode::Dual(SingleContentType::JSON, StreamContentType::SSE),
            ),
        ];

        for (accept_header, excepted_agreed) in cases {
            let agreed = negotiate_content_type(Some(accept_header))
                .expect("unable to parse accept header")
                .expect("no agreed response mode");
            assert_eq!(
                agreed, excepted_agreed,
                "wrong agreed response mode when negotiating: {}",
                accept_header
            );
        }
    }
}
