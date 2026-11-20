use crate::executor::projection::defer::IncrementalFormat;
use crate::pipeline::multipart_subscribe::{
    INCREMENTAL_DELIVERY_CONTENT_TYPE, INCREMENTAL_DELIVERY_DEFER_CONTENT_TYPE,
    INCREMENTAL_DELIVERY_LEGACY_DEFER_CONTENT_TYPE,
};
use crate::telemetry::logging::targets;
use headers_accept::Accept;
use http::{header::ACCEPT, Method};
use mediatype::{
    names::{HTML, TEXT},
    MediaType, Name, ReadParams, WriteParams,
};
use ntex::web::HttpRequest;
use std::str::FromStr;
use std::sync::LazyLock;
use strum::{AsRefStr, EnumIter, EnumString, IntoEnumIterator, IntoStaticStr};
use tracing::warn;

use crate::pipeline::error::{ClientPipelineError, PipelineError};

/// Non-GraphQL content type, used to detect if the client can accept Laboratory responses.
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
    /// Render the Laboratory IDE for the client. Used when the client prefers accepting HTML responses.
    /// It is different from the other modes because it does not represent a GraphQL response mode.
    Laboratory,
}

// `#[default]` attribute may only be used on unit enum variants, so we have to implement it
impl Default for ResponseMode {
    fn default() -> Self {
        ResponseMode::SingleOnly(SingleContentType::default())
    }
}

impl ResponseMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResponseMode::SingleOnly(_) => "single",
            ResponseMode::StreamOnly(_) => "stream",
            ResponseMode::Dual(_, _) => "dual",
            ResponseMode::Laboratory => "laboratory",
        }
    }

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
    /**
     * Whether to prefer HTTP 200 OK status code for errors.
     */
    pub fn prefer_status_ok_for_errors(&self) -> bool {
        matches!(
            self,
            ResponseMode::SingleOnly(SingleContentType::JSON)
                | ResponseMode::Dual(SingleContentType::JSON, _)
        )
    }
}

pub trait RequestAccepts {
    /// Reads the request's `Accept` header and returns the agreed response mode.
    ///
    /// When `laboratory_enabled` is `true`, a GET request that prefers `text/html`
    /// (e.g. a browser) negotiates to [`ResponseMode::Laboratory`]. When it is
    /// `false`, the `text/html` preference is ignored and negotiation falls through
    /// to the regular GraphQL content types so browsers still get a valid response.
    ///
    /// Returns an error if no valid content types are found in the Accept header.
    fn negotiate(&self, laboratory_enabled: bool) -> Result<ResponseMode, PipelineError>;
}

impl RequestAccepts for HttpRequest {
    #[inline]
    /// Reads the `Accept` header contents and returns a tuple of accepted/parsed content types.
    /// It perform negotiation and respects q-weights.
    fn negotiate(&self, laboratory_enabled: bool) -> Result<ResponseMode, PipelineError> {
        let accept_header = self
            .headers()
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();

        let accept_header = if accept_header.is_empty() {
            "*/*" // no header is same as this, but we want headers_accept to do the negotiation to be consistent
        } else {
            accept_header
        };

        let accept = Accept::from_str(accept_header).map_err(|err| {
            warn!(target: targets::HTTP_SERVER, error = ?err, "failed to parse Accept header");

            ClientPipelineError::InvalidHeaderValue(ACCEPT)
        })?;

        // strip multipart/mixed boundaries before negotiation since the server chooses the response boundary
        // as per the spec. furthermore, the incremental delivery and apollo multipart specs already have
        // their boundaries defined in the spec and we use those boundaries when generating the response
        // the same goes for the `@defer` format params, `defer_format` reads those from the header.
        // an entry asking for a `@defer` format version we don't speak is left out, as if it wasn't there
        let stripped = [
            Name::new_unchecked("boundary"),
            Name::new_unchecked("deferSpec"),
            Name::new_unchecked("incrementalSpec"),
        ];
        let accept = if accept.media_types().any(|media_type| {
            is_multipart_mixed(&media_type.to_ref())
                && stripped
                    .iter()
                    .any(|name| media_type.get_param(*name).is_some())
        }) {
            accept
                .media_types()
                .filter(|media_type| {
                    let media_type = media_type.to_ref();
                    !is_multipart_mixed(&media_type)
                        || is_subscription_spec(&media_type)
                        || multipart_defer_format(&media_type).is_some()
                })
                .map(|media_type| {
                    let mut media_type = media_type.to_ref();
                    if is_multipart_mixed(&media_type) {
                        for name in stripped {
                            media_type.remove_params(name);
                        }
                    }
                    media_type
                })
                .collect::<Accept>()
        } else {
            accept
        };

        if laboratory_enabled && self.method() == Method::GET {
            // if the client GETs we negotiate with the all supported media type, including HTML
            // to see if the client wants Laboratory. we negotiate with everything because browsers
            // tend to send very broad accept headers that include text/html with highest q-weight,
            // but would also accept */* which we would interpret as "I want normal GraphQL responses"
            let has_agreed_laboratory = accept
                .negotiate(ALL_RESPONSE_MODES_CONTENT_TYPE_MEDIA_TYPES.iter())
                .is_some_and(|t| *t == HTML_MEDIA_TYPE);
            if has_agreed_laboratory {
                return Ok(ResponseMode::Laboratory);
            }
        }

        let agreed_single: Option<SingleContentType> = accept
            .negotiate(SingleContentType::media_types().iter())
            .and_then(|t| t.try_into().ok()); // we dont care about the conversion error, it _should_ not happen

        let agreed_stream = accept
            .negotiate(StreamContentType::media_types().iter())
            .and_then(|t| t.try_into().ok()); // we dont care about the conversion error, it _should_ not happen

        match (agreed_single, agreed_stream) {
            (Some(single), Some(stream)) => Ok(ResponseMode::Dual(single, stream)),
            (Some(single), None) => Ok(ResponseMode::SingleOnly(single)),
            (None, Some(stream)) => Ok(ResponseMode::StreamOnly(stream)),
            // at this point we treat no content type as "user explicitly does not support any known types"
            // this is because only empty accept header or */* is treated as "accept everything" and we check
            // that above
            (None, None) => Err(ClientPipelineError::UnsupportedContentType.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use ntex::web::test::TestRequest;

    use super::*;

    #[test]
    fn negotiate_content_types() {
        // (method, accept header, laboratory enabled, expected mode)
        let cases = vec![
            (
                Method::GET,
                "",
                true,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::IncrementalDelivery,
                ),
            ),
            (
                Method::GET,
                "*/*",
                true,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::IncrementalDelivery,
                ),
            ),
            (
                Method::GET,
                r#"application/json, text/event-stream, multipart/mixed;subscriptionSpec="1.0""#,
                true,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::ApolloMultipartHTTP,
                ),
            ),
            (
                Method::GET,
                // unquoted boundary param
                "multipart/mixed;boundary=graphql;subscriptionSpec=1.0,application/json",
                true,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::ApolloMultipartHTTP,
                ),
            ),
            (
                Method::GET,
                // quoted boundary param
                "multipart/mixed;boundary=\"graphql\";subscriptionSpec=1.0,application/json",
                true,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::ApolloMultipartHTTP,
                ),
            ),
            (
                Method::GET,
                r#"application/graphql-response+json, multipart/mixed;q=0.5, text/event-stream;q=1"#,
                true,
                ResponseMode::Dual(
                    SingleContentType::GraphQLResponseJSON,
                    StreamContentType::SSE,
                ),
            ),
            (
                Method::GET,
                r#"application/json;q=0.5, application/graphql-response+json;q=1"#,
                true,
                ResponseMode::SingleOnly(SingleContentType::GraphQLResponseJSON),
            ),
            (
                Method::GET,
                r#"text/event-stream;q=0.5, multipart/mixed;q=1;subscriptionSpec="1.0""#,
                true,
                ResponseMode::StreamOnly(StreamContentType::ApolloMultipartHTTP),
            ),
            (
                Method::GET,
                r#"text/event-stream, application/json"#,
                true,
                ResponseMode::Dual(SingleContentType::JSON, StreamContentType::SSE),
            ),
            (
                // actual browser request loading a page
                Method::GET,
                r#"text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"#,
                true,
                ResponseMode::Laboratory,
            ),
            (
                // browser accept header snippet but for a POST request
                Method::POST,
                r#"text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"#,
                true,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::IncrementalDelivery,
                ),
            ),
            (
                // browser GET while Laboratory is disabled: the `text/html` preference is
                // ignored and the broad `*/*` browsers send is honored, so the client gets a
                // normal GraphQL response instead of a 404.
                Method::GET,
                r#"text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"#,
                false,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::IncrementalDelivery,
                ),
            ),
            (
                // empty Accept on a GET with Laboratory disabled still negotiates normally.
                Method::GET,
                "",
                false,
                ResponseMode::Dual(
                    SingleContentType::JSON,
                    StreamContentType::IncrementalDelivery,
                ),
            ),
            (
                // explicit GraphQL content type on a GET is unaffected by Laboratory being disabled.
                Method::GET,
                r#"application/graphql-response+json"#,
                false,
                ResponseMode::SingleOnly(SingleContentType::GraphQLResponseJSON),
            ),
        ];

        for (method, accept_header, laboratory_enabled, excepted_agreed) in cases {
            let request = TestRequest::default()
                .method(method.clone())
                .header(ACCEPT, accept_header)
                .to_http_request();
            let agreed = request
                .negotiate(laboratory_enabled)
                .expect("unable to parse accept header");
            assert_eq!(
                agreed, excepted_agreed,
                "wrong agreed response mode when negotiating method {} with accept: {} (laboratory_enabled={})",
                method, accept_header, laboratory_enabled
            );
        }
    }

    #[test]
    fn browser_get_with_only_html_and_laboratory_disabled_is_unsupported() {
        // A client that GETs accepting *only* `text/html` (no `*/*` fallback) while
        // Laboratory is disabled has no acceptable GraphQL content type, so negotiation
        // fails with an explicit unsupported-content-type error rather than a misleading 404.
        let request = TestRequest::default()
            .method(Method::GET)
            .header(ACCEPT, "text/html")
            .to_http_request();
        let result = request.negotiate(false);
        assert!(
            matches!(
                result,
                Err(PipelineError::Client(
                    ClientPipelineError::UnsupportedContentType
                ))
            ),
            "expected UnsupportedContentType, got {:?}",
            result
        );
    }

    #[test]
    fn multipart_with_a_defer_version_we_dont_speak_is_left_out() {
        let negotiate = |accept: &str| {
            TestRequest::default()
                .method(Method::POST)
                .header(ACCEPT, accept)
                .to_http_request()
                .negotiate(false)
        };
        // the other types still negotiate
        assert_eq!(
            negotiate("multipart/mixed;incrementalSpec=v0.3, application/json").unwrap(),
            ResponseMode::SingleOnly(SingleContentType::JSON)
        );
        assert_eq!(
            negotiate("multipart/mixed;deferSpec=20230101, text/event-stream").unwrap(),
            ResponseMode::StreamOnly(StreamContentType::SSE)
        );
        // a supported one next to it still counts
        assert_eq!(
            negotiate("multipart/mixed;incrementalSpec=v0.3, multipart/mixed;deferSpec=20220824")
                .unwrap(),
            ResponseMode::StreamOnly(StreamContentType::IncrementalDelivery)
        );
        // nothing left is nothing acceptable
        let result = negotiate("multipart/mixed;incrementalSpec=v0.3");
        assert!(
            matches!(
                result,
                Err(PipelineError::Client(
                    ClientPipelineError::UnsupportedContentType
                ))
            ),
            "expected UnsupportedContentType, got {result:?}"
        );
    }

    #[test]
    fn html_only_get_renders_laboratory_when_enabled() {
        // The same `text/html`-only GET renders Laboratory when it is enabled.
        let request = TestRequest::default()
            .method(Method::GET)
            .header(ACCEPT, "text/html")
            .to_http_request();
        let agreed = request
            .negotiate(true)
            .expect("unable to parse accept header");
        assert_eq!(agreed, ResponseMode::Laboratory);
    }
}

fn is_multipart_mixed(media_type: &MediaType) -> bool {
    media_type.ty.as_str() == "multipart" && media_type.subty.as_str() == "mixed"
}

fn is_subscription_spec(media_type: &MediaType) -> bool {
    media_type
        .get_param(Name::new_unchecked("subscriptionSpec"))
        .is_some()
}

/// The `@defer` format a `multipart/mixed` entry asks for. No version means the current spec.
/// `None` for a version we don't speak, or for both params at once.
fn multipart_defer_format(media_type: &MediaType) -> Option<IncrementalFormat> {
    let defer_spec = media_type.get_param(Name::new_unchecked("deferSpec"));
    let incremental_spec = media_type.get_param(Name::new_unchecked("incrementalSpec"));
    match (defer_spec, incremental_spec) {
        (None, None) => Some(IncrementalFormat::Spec),
        (Some(value), None) if value == "20220824" => Some(IncrementalFormat::Legacy),
        (None, Some(value)) if value == "v0.2" => Some(IncrementalFormat::Spec),
        _ => None,
    }
}

/// The `@defer` format for a response that negotiated `multipart/mixed`, from the same `Accept`
/// value `negotiate` reads. Of the `multipart/mixed` entries with a format we speak and a `q`
/// above 0, the one with the highest `q` wins, the first one on a tie. The current spec when
/// there's none, like for `*/*`.
pub fn defer_format(accept_header: &str) -> IncrementalFormat {
    defer_entry(accept_header)
        .map(|(format, _)| format)
        .unwrap_or_default()
}

/// The content type of a `multipart/mixed` response with nothing deferred, the whole response
/// is its only part. It keeps the `@defer` version the client asked for, like when something is
/// deferred, so `@defer(if:)` doesn't change it. Plain `multipart/mixed` when it asked for none.
pub fn single_part_content_type(accept_header: &str) -> &'static str {
    match defer_entry(accept_header) {
        Some((IncrementalFormat::Legacy, _)) => INCREMENTAL_DELIVERY_LEGACY_DEFER_CONTENT_TYPE,
        Some((IncrementalFormat::Spec, true)) => INCREMENTAL_DELIVERY_DEFER_CONTENT_TYPE,
        _ => INCREMENTAL_DELIVERY_CONTENT_TYPE,
    }
}

/// The `multipart/mixed` entry [`defer_format`] picks: its format, and whether it names a
/// version. `None` when there's no such entry.
fn defer_entry(accept_header: &str) -> Option<(IncrementalFormat, bool)> {
    let accept = Accept::from_str(accept_header).ok()?;
    let q = |media_type: &MediaType| {
        media_type
            .get_param(Name::new_unchecked("q"))
            .and_then(|value| value.as_str().parse::<f32>().ok())
            .unwrap_or(1.0)
    };
    let mut best: Option<(f32, IncrementalFormat, bool)> = None;
    for media_type in accept.media_types() {
        let media_type = media_type.to_ref();
        if !is_multipart_mixed(&media_type) || is_subscription_spec(&media_type) {
            continue;
        }
        let q = q(&media_type);
        if q <= 0.0 {
            continue;
        }
        if let Some(format) = multipart_defer_format(&media_type) {
            if best.is_none_or(|(best_q, _, _)| q > best_q) {
                let named = media_type
                    .get_param(Name::new_unchecked("deferSpec"))
                    .or_else(|| media_type.get_param(Name::new_unchecked("incrementalSpec")))
                    .is_some();
                best = Some((q, format, named));
            }
        }
    }
    best.map(|(_, format, named)| (format, named))
}

#[cfg(test)]
mod defer_accept_tests {
    use super::defer_format;
    use crate::executor::projection::defer::IncrementalFormat::{Legacy, Spec};

    #[test]
    fn single_part_keeps_the_version() {
        let cases = [
            (
                "multipart/mixed;deferSpec=20220824",
                r#"multipart/mixed;boundary="-";deferSpec=20220824"#,
            ),
            (
                "multipart/mixed;incrementalSpec=v0.2",
                r#"multipart/mixed;boundary="-";incrementalSpec=v0.2"#,
            ),
            ("multipart/mixed", r#"multipart/mixed;boundary="-""#),
            ("*/*", r#"multipart/mixed;boundary="-""#),
            (
                r#"multipart/mixed;subscriptionSpec="1.0""#,
                r#"multipart/mixed;boundary="-""#,
            ),
        ];
        for (accept, expected) in cases {
            assert_eq!(
                super::single_part_content_type(accept),
                expected,
                "{accept}"
            );
        }
    }

    #[test]
    fn picks_the_defer_format() {
        let cases = [
            ("multipart/mixed;deferSpec=20220824, application/json", Legacy),
            (r#"application/json, multipart/mixed; deferSpec="20220824""#, Legacy),
            ("multipart/mixed;incrementalSpec=v0.2", Spec),
            // no version is the current spec
            ("multipart/mixed", Spec),
            ("*/*", Spec),
            ("application/json;deferSpec=20220824", Spec),
            // the highest `q` wins
            (
                "multipart/mixed;incrementalSpec=v0.2;q=1, multipart/mixed;deferSpec=20220824;q=0.5",
                Spec,
            ),
            (
                "multipart/mixed;incrementalSpec=v0.2;q=0.5, multipart/mixed;deferSpec=20220824",
                Legacy,
            ),
            // `q=0` is "not this one"
            ("multipart/mixed;deferSpec=20220824;q=0, multipart/mixed", Spec),
            // the first one on a tie
            (
                "multipart/mixed;deferSpec=20220824, multipart/mixed;incrementalSpec=v0.2",
                Legacy,
            ),
            // a version we don't speak doesn't count
            ("multipart/mixed;incrementalSpec=v0.3, multipart/mixed;deferSpec=20220824;q=0.1", Legacy),
            ("multipart/mixed;deferSpec=20230101;q=1, multipart/mixed;deferSpec=20220824;q=0.1", Legacy),
        ];
        for (accept, expected) in cases {
            assert_eq!(defer_format(accept), expected, "{accept}");
        }
    }
}
