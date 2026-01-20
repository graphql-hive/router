use headers_accept::Accept;
use http::header::ACCEPT;
use mediatype::{MediaType, Name, ReadParams};
use ntex::web::HttpRequest;
use once_cell::sync::Lazy;
use std::str::FromStr;
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator, IntoStaticStr};
use tracing::error;

use crate::pipeline::error::PipelineError;

/// Non-GraphQL content type, used to detect if the client can accept GraphiQL responses.
pub const TEXT_HTML_MIME: &str = "text/html";

// IMPORTANT: make sure that the serialized string representations are valid because
//            there is an unwrap in the SingleContentType::media_types() method.
/// Non-streamable (single) content types for GraphQL responses.
#[derive(PartialEq, Default, Debug, Clone, IntoStaticStr, EnumString, AsRefStr, EnumIter)]
pub enum SingleContentType {
    // The order of the variants here matters for negotiation with `Accept: */*`.
    /// Legacy GraphQL over HTTP (`application/json`)
    ///
    /// Default for regular queries and mutations.
    ///
    /// Read more: https://graphql.github.io/graphql-over-http
    #[default]
    #[strum(serialize = "application/json")]
    JSON,
    /// GraphQL over HTTP spec (`application/graphql-response+json`)
    ///
    /// Read more: https://graphql.github.io/graphql-over-http
    #[strum(serialize = "application/graphql-response+json")]
    GraphQLResponseJSON,
}

impl TryFrom<&MediaType<'_>> for SingleContentType {
    type Error = &'static str;

    /// The only thing where the conversion can fail is if the media type is not supported.
    fn try_from(media_type: &MediaType) -> Result<Self, Self::Error> {
        let ty = media_type.ty.as_str();
        let subty = media_type.subty.as_str();
        let suffix = media_type.suffix.as_ref().map(|s| s.as_str());

        if ty == "application" {
            if subty == "graphql-response" && suffix == Some("json") {
                return Ok(SingleContentType::GraphQLResponseJSON);
            } else if subty == "json" && suffix.is_none() {
                return Ok(SingleContentType::JSON);
            }
        }

        Err("Unsupported single content type")
    }
}

static SINGLE_CONTENT_TYPE_MEDIA_TYPES: Lazy<Vec<MediaType<'static>>> = Lazy::new(|| {
    // first collect the string representations to keep them alive
    // in order to parse them into MediaType instances that _borrow_
    // the items from the vec
    let strs: Vec<&'static str> = SingleContentType::iter().map(|ct| ct.into()).collect();
    strs.iter()
        .map(|s| {
            MediaType::parse(s)
                // SAFETY: we control the strings being parsed here. see the enum variants
                .unwrap()
        })
        .collect()
});

impl SingleContentType {
    // no consts until https://github.com/picoHz/mediatype/pull/25 lands
    pub fn media_types() -> &'static Vec<MediaType<'static>> {
        &SINGLE_CONTENT_TYPE_MEDIA_TYPES
    }
}

// IMPORTANT: make sure that the serialized string representations are valid because
//            there is an unwrap in the StreamContentType::media_types() method.
/// Streamable content types for GraphQL responses.
#[derive(PartialEq, Default, Debug, IntoStaticStr, EnumString, AsRefStr, EnumIter)]
pub enum StreamContentType {
    // The order of the variants here matters for negotiation with `Accept: */*`.
    /// Incremental Delivery over HTTP (`multipart/mixed`)
    ///
    /// Default for subscriptions.
    ///
    /// Read more: https://github.com/graphql/graphql-over-http/blob/c144dbd89cbea6bde0045205e34e01002f9f9ba0/rfcs/IncrementalDelivery.md
    #[default]
    #[strum(serialize = "multipart/mixed")]
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
    #[strum(serialize = r#"multipart/mixed;subscriptionSpec="1.0""#)]
    ApolloMultipartHTTP,
}

impl TryFrom<&MediaType<'_>> for StreamContentType {
    type Error = &'static str;

    /// The only thing where the conversion can fail is if the media type is not supported.
    fn try_from(media_type: &MediaType) -> Result<Self, Self::Error> {
        let ty = media_type.ty.as_str();
        let subty = media_type.subty.as_str();

        if ty == "multipart" && subty == "mixed" {
            if media_type
                .get_param(Name::new_unchecked("subscriptionSpec"))
                .is_some_and(|s| s.unquoted_str() == "1.0")
            {
                return Ok(StreamContentType::ApolloMultipartHTTP);
            } else {
                return Ok(StreamContentType::IncrementalDelivery);
            }
        } else if ty == "text" && subty == "event-stream" {
            return Ok(StreamContentType::SSE);
        }

        Err("Unsupported stream content type")
    }
}

static STREAM_CONTENT_TYPE_MEDIA_TYPES: Lazy<Vec<MediaType<'static>>> = Lazy::new(|| {
    // first collect the string representations to keep them alive
    // in order to parse them into MediaType instances that _borrow_
    // the items from the vec
    let strs: Vec<&'static str> = StreamContentType::iter().map(|ct| ct.into()).collect();
    strs.iter()
        .map(|s| {
            MediaType::parse(s)
                // SAFETY: we control the strings being parsed here. see the enum variants
                .unwrap()
        })
        .collect()
});

impl StreamContentType {
    // no consts until https://github.com/picoHz/mediatype/pull/25 lands
    pub fn media_types() -> &'static Vec<MediaType<'static>> {
        &STREAM_CONTENT_TYPE_MEDIA_TYPES
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
    let accept_header = if accept_header.is_empty() {
        "*/*" // no header is same as this, but we want headers_accept to do the negotiation to be consistent
    } else {
        accept_header
    };

    let accept = Accept::from_str(accept_header)?;

    let agreed_single: Option<SingleContentType> = accept
        .negotiate(SingleContentType::media_types().iter())
        .and_then(|t| t.try_into().ok()); // we dont care about the conversion error, it _should_ not happen

    let agreed_stream = accept
        .negotiate(StreamContentType::media_types().iter())
        .and_then(|t| t.try_into().ok()); // we dont care about the conversion error, it _should_ not happen

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
                "*/*",
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
