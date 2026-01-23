use headers_accept::Accept;
use http::{header::ACCEPT, Method};
use mediatype::{
    names::{HTML, TEXT},
    MediaType, Name, ReadParams,
};
use ntex::web::HttpRequest;
use std::str::FromStr;
use std::sync::LazyLock;
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

static SINGLE_CONTENT_TYPE_MEDIA_TYPES: LazyLock<Vec<MediaType<'static>>> = LazyLock::new(|| {
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

static STREAM_CONTENT_TYPE_MEDIA_TYPES: LazyLock<Vec<MediaType<'static>>> = LazyLock::new(|| {
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

const HTML_MEDIA_TYPE: MediaType<'static> = MediaType::new(TEXT, HTML);

static ALL_RESPONSE_MODES_CONTENT_TYPE_MEDIA_TYPES: LazyLock<Vec<MediaType<'static>>> =
    LazyLock::new(|| {
        let mut all_media_types = Vec::with_capacity(
            1 + SINGLE_CONTENT_TYPE_MEDIA_TYPES.len() + STREAM_CONTENT_TYPE_MEDIA_TYPES.len(),
        );
        all_media_types.extend(SINGLE_CONTENT_TYPE_MEDIA_TYPES.iter().cloned());
        all_media_types.extend(STREAM_CONTENT_TYPE_MEDIA_TYPES.iter().cloned());
        all_media_types.push(HTML_MEDIA_TYPE); // must be last for negotiation priority
        all_media_types
    });

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
    /// Render the GraphiQL IDE for the client. Used when the client prefers accepting HTML responses.
    /// It is different from the other modes because it does not represent a GraphQL response mode.
    GraphiQL,
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
            _ => None,
        }
    }
    pub fn can_stream(&self) -> bool {
        matches!(self, ResponseMode::StreamOnly(_) | ResponseMode::Dual(_, _))
    }
    pub fn stream_content_type(&self) -> Option<&StreamContentType> {
        match self {
            ResponseMode::StreamOnly(stream) => Some(stream),
            ResponseMode::Dual(_, stream) => Some(stream),
            _ => None,
        }
    }
}

/// Reads the `Accept` header contents and returns a tuple of accepted/parsed content types.
/// It perform negotiation and respects q-weights.
fn negotiate_content_type(
    method: &Method,
    accept_header: Option<&str>,
) -> Result<Option<ResponseMode>, <Accept as FromStr>::Err> {
    let accept_header = accept_header.unwrap_or_default();
    let accept_header = if accept_header.is_empty() {
        "*/*" // no header is same as this, but we want headers_accept to do the negotiation to be consistent
    } else {
        accept_header
    };

    let accept = Accept::from_str(accept_header)?;

    if method == Method::GET {
        // if the client GETs we negotiate with the all supported media type, including HTML
        // to see if the client wants GraphiQL. we negotiate with everything because browsers
        // tend to send very broad accept headers that include text/html with highest q-weight,
        // but would also accept */* which we would interpret as "I want normal GraphQL responses"
        let has_agreed_graphiql = accept
            .negotiate(ALL_RESPONSE_MODES_CONTENT_TYPE_MEDIA_TYPES.iter())
            .is_some_and(|t| *t == HTML_MEDIA_TYPE);
        if has_agreed_graphiql {
            return Ok(Some(ResponseMode::GraphiQL));
        }
    }

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
    /// Reads the request's `Accept` header and returns the agreed response mode.
    ///
    /// Returns an error if no valid content types are found in the Accept header.
    fn negotiate(&self) -> Result<ResponseMode, PipelineError>;
}

impl RequestAccepts for HttpRequest {
    #[inline]
    fn negotiate(&self) -> Result<ResponseMode, PipelineError> {
        let content_types = self
            .headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok());

        let agreed = negotiate_content_type(self.method(), content_types).map_err(|err| {
            error!("Failed to parse Accept header: {}", err);
            PipelineError::InvalidHeaderValue(ACCEPT)
        })?;

        // at this point we treat no content type as "user explicitly does not support any known types"
        // this is because only empty accept header or */* is treated as "accept everything" and we check
        // that above
        match agreed {
            Some(response_mode) => Ok(response_mode),
            None => Err(PipelineError::UnsupportedContentType),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiate_content_types() {
        let cases = vec![
            (
                Method::GET,
                "",
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::IncrementalDelivery,
                ),
            ),
            (
                Method::GET,
                "*/*",
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::IncrementalDelivery,
                ),
            ),
            (
                Method::GET,
                r#"application/json, text/event-stream, multipart/mixed;subscriptionSpec="1.0""#,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::ApolloMultipartHTTP,
                ),
            ),
            (
                Method::GET,
                r#"application/graphql-response+json, multipart/mixed;q=0.5, text/event-stream;q=1"#,
                ResponseMode::Dual(
                    SingleContentType::GraphQLResponseJSON,
                    StreamContentType::SSE,
                ),
            ),
            (
                Method::GET,
                r#"application/json;q=0.5, application/graphql-response+json;q=1"#,
                ResponseMode::SingleOnly(SingleContentType::GraphQLResponseJSON),
            ),
            (
                Method::GET,
                r#"text/event-stream;q=0.5, multipart/mixed;q=1;subscriptionSpec="1.0""#,
                ResponseMode::StreamOnly(StreamContentType::ApolloMultipartHTTP),
            ),
            (
                Method::GET,
                r#"text/event-stream, application/json"#,
                ResponseMode::Dual(SingleContentType::JSON, StreamContentType::SSE),
            ),
            (
                // actual browser request loading a page
                Method::GET,
                r#"text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"#,
                ResponseMode::GraphiQL,
            ),
            (
                // browser accept header snippet but for a POST request
                Method::POST,
                r#"text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"#,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::IncrementalDelivery,
                ),
            ),
        ];

        for (method, accept_header, excepted_agreed) in cases {
            let agreed = negotiate_content_type(&method, Some(accept_header))
                .expect("unable to parse accept header")
                .expect("no agreed response mode");
            assert_eq!(
                agreed, excepted_agreed,
                "wrong agreed response mode when negotiating method {} with accept: {}",
                method, accept_header
            );
        }
    }
}
