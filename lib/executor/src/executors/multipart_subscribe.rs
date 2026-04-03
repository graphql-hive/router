use std::str::Utf8Error;

use bytes::{Buf, Bytes};
use futures::stream::BoxStream;
use http_body_util::BodyExt;
use hyper::body::Body;

use crate::{
    executors::error::SubgraphExecutorError, response::subgraph_response::SubgraphResponse,
};

// subgraphs are internal and generally safe, but this limit exists as defense-in-depth
// to prevent a misbehaving subgraph from growing the buffer unboundedly until OOM
const MAX_BUFFER_SIZE: usize = 10 * 1024 * 1024; // 10MB

#[derive(thiserror::Error, Debug)]
pub enum ParseError {
    #[error("Invalid UTF-8 sequence: {0}")]
    InvalidUtf8(#[from] Utf8Error),
    #[error("Stream read error: {0}")]
    StreamReadError(String),
    #[error("Invalid subgraph response: {0}")]
    InvalidSubgraphResponse(SubgraphExecutorError),
    #[error("Missing boundary parameter in Content-Type header")]
    MissingBoundary,
    #[error("Invalid boundary parameter: {0}")]
    InvalidBoundary(String),
    #[error("Buffer size limit exceeded: stream sent more than {MAX_BUFFER_SIZE} bytes without a part boundary")]
    BufferSizeLimitExceeded,
}

/// Parse the boundary parameter from a Content-Type header value.
///
/// Example: `multipart/mixed; boundary=graphql` returns `Ok("graphql")`
///
/// Returns an error if:
/// - The boundary parameter is missing (boundary is required by the spec)
/// - The boundary value is empty (boundary is required by the spec)
/// - The quoted boundary is not properly closed
pub fn parse_boundary_from_header(content_type: &str) -> Result<&str, ParseError> {
    let content_type = content_type.trim();

    let content_type_lower = content_type.to_lowercase();
    let boundary_param_start = content_type_lower
        .find("boundary=")
        .ok_or(ParseError::MissingBoundary)?;
    let value_start = boundary_param_start + "boundary=".len();

    let remaining = &content_type[value_start..];

    let boundary = if let Some(inside) = remaining.strip_prefix('"') {
        // quoted boundary: find the closing quote
        let quote_end = inside
            .find('"')
            .ok_or_else(|| ParseError::InvalidBoundary("Unclosed quoted boundary".to_string()))?;
        &remaining[1..quote_end + 1]
    } else {
        // unquoted boundary: value extends until semicolon, whitespace, or end of string
        let end = remaining
            .find(|c: char| c == ';' || c.is_whitespace())
            .unwrap_or(remaining.len());
        &remaining[..end]
    };

    // boundary most not empty
    if boundary.is_empty() {
        return Err(ParseError::InvalidBoundary(
            "Empty boundary value".to_string(),
        ));
    }

    Ok(boundary)
}

pub fn parse_to_stream<B>(
    boundary: &str,
    body_stream: B,
) -> BoxStream<'static, Result<SubgraphResponse<'static>, ParseError>>
where
    B: Body + Send + Unpin + 'static,
    B::Data: Buf + Send,
    B::Error: std::fmt::Display + Send,
{
    let delimiter = format!("--{}", boundary);
    let end_marker = format!("--{}--", boundary);

    let stream = async_stream::stream! {
        let mut body = body_stream;
        let mut buffer = Vec::<u8>::new();
        let mut started = false;

        loop {
            while let Some((part_end, skip_len, is_end)) = find_next_part(&buffer, &delimiter, &end_marker, started) {
                if !started {
                    buffer.drain(..skip_len);
                    started = true;
                    continue;
                }

                let part_bytes: Vec<u8> = buffer.drain(..part_end).collect();
                buffer.drain(..skip_len);

                if !part_bytes.is_empty() {
                    match parse_part(&part_bytes) {
                        Ok(Some(response)) => {
                            yield Ok(response);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            yield Err(e);
                            return;
                        }
                    }
                }

                if is_end {
                    return;
                }
            }

            match body.frame().await {
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        buffer.extend_from_slice(data.chunk());
                        if buffer.len() > MAX_BUFFER_SIZE {
                            yield Err(ParseError::BufferSizeLimitExceeded);
                            return;
                        }
                    }
                }
                Some(Err(e)) => {
                    yield Err(ParseError::StreamReadError(e.to_string()));
                    return;
                }
                None => {
                    return;
                }
            }
        }
    };

    Box::pin(stream)
}

fn find_next_part(
    buffer: &[u8],
    delimiter: &str,
    end_marker: &str,
    started: bool,
) -> Option<(usize, usize, bool)> {
    let delimiter_b = delimiter.as_bytes();
    let end_marker_b = end_marker.as_bytes();

    let find_bytes = |haystack: &[u8], needle: &[u8]| -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    };

    if !started {
        let pos = find_bytes(buffer, delimiter_b)?;
        let after_delimiter = pos + delimiter_b.len();
        let newline_pos = buffer[after_delimiter..].iter().position(|&b| b == b'\n')?;
        let skip_len = after_delimiter + newline_pos + 1;
        return Some((0, skip_len, false));
    }

    let next_delimiter_pos = find_bytes(buffer, delimiter_b)?;
    let is_end = buffer[next_delimiter_pos..].starts_with(end_marker_b);

    let skip_len = if is_end {
        let after_end = next_delimiter_pos + end_marker_b.len();
        if let Some(newline) = buffer[after_end..].iter().position(|&b| b == b'\n') {
            end_marker_b.len() + newline + 1
        } else {
            end_marker_b.len()
        }
    } else {
        let after_delimiter = next_delimiter_pos + delimiter_b.len();
        if let Some(newline) = buffer[after_delimiter..].iter().position(|&b| b == b'\n') {
            delimiter_b.len() + newline + 1
        } else {
            delimiter_b.len()
        }
    };

    Some((next_delimiter_pos, skip_len, is_end))
}

fn parse_part(raw: &[u8]) -> Result<Option<SubgraphResponse<'static>>, ParseError> {
    let text = std::str::from_utf8(raw)?;
    let body = extract_body_after_headers(text);

    if body.is_empty() {
        return Ok(None);
    }

    extract_payload(body)
}

fn extract_body_after_headers(content: &str) -> &str {
    if let Some(pos) = content.find("\r\n\r\n") {
        content[pos + 4..].trim()
    } else if let Some(pos) = content.find("\n\n") {
        content[pos + 2..].trim()
    } else {
        content.trim()
    }
}

fn extract_payload(body: &str) -> Result<Option<SubgraphResponse<'static>>, ParseError> {
    // cheap heartbeat check: subgraphs send `{}` as a keep-alive ping
    if body == "{}" {
        return Ok(None);
    }

    let payload_lv = sonic_rs::get_from_str(body, &["payload"]);

    match payload_lv {
        Ok(lv) => {
            let raw = lv.as_raw_str();

            if raw == "null" {
                // transport error: payload is null, check for top-level errors
                if let Ok(errors_lv) = sonic_rs::get_from_str(body, &["errors"]) {
                    let transport_err = format!(r#"{{"errors":{}}}"#, errors_lv.as_raw_str());
                    return SubgraphResponse::deserialize_from_bytes(Bytes::from(transport_err))
                        .map_err(ParseError::InvalidSubgraphResponse)
                        .map(Some);
                }
                return Ok(None);
            }

            // happy path: deserialize the raw payload substring directly - no re-serialization
            SubgraphResponse::deserialize_from_bytes(Bytes::copy_from_slice(raw.as_bytes()))
                .map_err(ParseError::InvalidSubgraphResponse)
                .map(Some)
        }
        Err(e) if e.is_not_found() => {
            // no payload wrapper, treat the whole body as a subgraph response
            SubgraphResponse::deserialize_from_bytes(Bytes::from(body.to_owned()))
                .map_err(ParseError::InvalidSubgraphResponse)
                .map(Some)
        }
        Err(e) => Err(ParseError::InvalidSubgraphResponse(
            SubgraphExecutorError::ResponseDeserializationFailure(e),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_parse_boundary_from_header_simple() {
        let content_type = "multipart/mixed; boundary=graphql";
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "graphql");
    }

    #[test]
    fn test_parse_boundary_from_header_quoted() {
        let content_type = r#"multipart/mixed; boundary="my-boundary""#;
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "my-boundary");
    }

    #[test]
    fn test_parse_boundary_from_header_with_spaces() {
        let content_type = "multipart/mixed;   boundary=graphql";
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "graphql");
    }

    #[test]
    fn test_parse_boundary_from_header_with_additional_params() {
        let content_type = r#"multipart/mixed; boundary=graphql; charset=utf-8"#;
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "graphql");
    }

    #[test]
    fn test_parse_boundary_from_header_quoted_with_additional_params() {
        let content_type = r#"multipart/mixed; boundary="my-boundary"; charset=utf-8"#;
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "my-boundary");
    }

    #[test]
    fn test_parse_boundary_from_header_complex_boundary() {
        let content_type = "multipart/mixed; boundary=----WebKitFormBoundary7MA4YWxkTrZu0gW";
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "----WebKitFormBoundary7MA4YWxkTrZu0gW");
    }

    #[test]
    fn test_parse_boundary_from_header_with_subscription_spec() {
        let content_type = r#"multipart/mixed; boundary=graphql; subscriptionSpec="1.0""#;
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "graphql");
    }

    #[test]
    fn test_parse_boundary_from_header_quoted_special_chars() {
        let content_type =
            r#"multipart/mixed; boundary="boundary-with-dashes_and_underscores.123""#;
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "boundary-with-dashes_and_underscores.123");
    }

    #[test]
    fn test_parse_boundary_from_header_no_boundary() {
        let content_type = "multipart/mixed";
        let boundary = parse_boundary_from_header(content_type);
        assert!(matches!(boundary, Err(ParseError::MissingBoundary)));
    }

    #[test]
    fn test_parse_boundary_from_header_empty_string() {
        let content_type = "";
        let boundary = parse_boundary_from_header(content_type);
        assert!(matches!(boundary, Err(ParseError::MissingBoundary)));
    }

    #[test]
    fn test_parse_boundary_from_header_case_insensitive() {
        let cases = [
            "multipart/mixed; Boundary=graphql",
            "multipart/mixed; BOUNDARY=graphql",
            "multipart/mixed; boundary=graphql",
            "multipart/mixed; bOuNdArY=graphql",
        ];
        for content_type in cases {
            let boundary = parse_boundary_from_header(content_type).unwrap();
            assert_eq!(boundary, "graphql", "failed for: {content_type}");
        }
    }

    #[test]
    fn test_parse_boundary_from_header_whitespace_in_boundary() {
        let content_type = "multipart/mixed; boundary=my boundary";
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "my");
    }

    #[test]
    fn test_parse_boundary_from_header_quoted_with_whitespace() {
        let content_type = r#"multipart/mixed; boundary="my boundary""#;
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "my boundary");
    }

    #[test]
    fn test_parse_boundary_from_header_dash_boundary() {
        let content_type = "multipart/mixed; boundary=-";
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "-");
    }

    #[test]
    fn test_parse_boundary_from_header_trailing_whitespace() {
        let content_type = "multipart/mixed; boundary=graphql   ";
        let boundary = parse_boundary_from_header(content_type).unwrap();
        assert_eq!(boundary, "graphql");
    }

    #[test]
    fn test_parse_boundary_from_header_empty_value() {
        let content_type = "multipart/mixed; boundary=";
        let boundary = parse_boundary_from_header(content_type);
        assert!(matches!(boundary, Err(ParseError::InvalidBoundary(_))));
    }

    #[test]
    fn test_parse_boundary_from_header_empty_quoted_value() {
        let content_type = r#"multipart/mixed; boundary="""#;
        let boundary = parse_boundary_from_header(content_type);
        assert!(matches!(boundary, Err(ParseError::InvalidBoundary(_))));
    }

    #[test]
    fn test_parse_boundary_from_header_unclosed_quote() {
        let content_type = r#"multipart/mixed; boundary="graphql"#;
        let boundary = parse_boundary_from_header(content_type);
        assert!(matches!(boundary, Err(ParseError::InvalidBoundary(_))));
    }

    #[test]
    fn test_parse_part_with_headers() {
        let part_data =
            b"Content-Type: application/json\r\n\r\n{\"payload\":{\"data\":{\"reviewAdded\":{\"id\":\"1\"}}}}";

        let response = parse_part(part_data)
            .expect("Should parse valid part")
            .expect("Should have response");

        assert!(!response.data.is_null());
    }

    #[test]
    fn test_parse_part_without_headers() {
        let part_data = b"\r\n{\"payload\":{\"data\":{\"value\":1}}}";

        let response = parse_part(part_data)
            .expect("Should parse part without headers")
            .expect("Should have response");

        assert!(!response.data.is_null());
    }

    #[test]
    fn test_extract_payload_with_payload_property() {
        let body = r#"{"payload":{"data":{"reviewAdded":{"id":"1"}}}}"#;

        let response = extract_payload(body)
            .expect("Should extract payload")
            .expect("Should have response");

        assert!(!response.data.is_null());
    }

    #[test]
    fn test_extract_payload_without_payload_property() {
        let body = r#"{"data":{"user":{"name":"Alice"}}}"#;

        let response = extract_payload(body)
            .expect("Should extract payload")
            .expect("Should have response");

        assert!(!response.data.is_null());
    }

    #[test]
    fn test_extract_payload_heartbeat() {
        let body = "{}";

        let payload = extract_payload(body).expect("Should parse heartbeat");

        assert!(payload.is_none(), "Heartbeat should return None");
    }

    #[test]
    fn test_extract_payload_transport_error() {
        let body = r#"{"payload":null,"errors":[{"message":"Connection lost"}]}"#;

        let response = extract_payload(body)
            .expect("Should extract error")
            .expect("Should have error response");

        assert!(response.errors.is_some());
        let errors = response.errors.unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Connection lost");
    }

    #[tokio::test]
    async fn test_parse_to_stream_single_event() {
        use futures::StreamExt;
        use http_body_util::StreamBody;
        use hyper::body::Frame;

        let chunks: Vec<Result<Frame<Bytes>, std::convert::Infallible>> = vec![Ok(Frame::data(
            Bytes::from(
                "--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{\"test\":1}}}\r\n--graphql--\r\n",
            ),
        ))];

        let body = StreamBody::new(futures::stream::iter(chunks));
        let mut stream = parse_to_stream("graphql", body);

        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let response = first_result.unwrap();
        assert!(!response.data.is_null());

        let second = stream.next().await;
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn test_parse_to_stream_chunked_events() {
        use futures::StreamExt;
        use http_body_util::StreamBody;
        use hyper::body::Frame;

        let chunks: Vec<Result<Frame<Bytes>, std::convert::Infallible>> = vec![
            Ok(Frame::data(Bytes::from(
                "--graphql\r\nContent-Type: application/json\r\n\r\n{\"pay",
            ))),
            Ok(Frame::data(Bytes::from(
                "load\":{\"data\":{\"value\":1}}}\r\n--graphql\r\n",
            ))),
            Ok(Frame::data(Bytes::from(
                "Content-Type: application/json\r\n\r\n{\"payload\":{\"data\":{\"value\":2}}}\r\n--graphql--\r\n",
            ))),
        ];

        let body = StreamBody::new(futures::stream::iter(chunks));
        let mut stream = parse_to_stream("graphql", body);

        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let response = first_result.unwrap();
        assert!(!response.data.is_null());

        let second = stream.next().await;
        assert!(second.is_some());
        let second_result = second.unwrap();
        assert!(second_result.is_ok());
        let response = second_result.unwrap();
        assert!(!response.data.is_null());

        let third = stream.next().await;
        assert!(third.is_none());
    }

    #[tokio::test]
    async fn test_parse_to_stream_with_heartbeat() {
        use futures::StreamExt;
        use http_body_util::StreamBody;
        use hyper::body::Frame;

        let chunks: Vec<Result<Frame<Bytes>, std::convert::Infallible>> = vec![Ok(Frame::data(
            Bytes::from(
                "--graphql\r\nContent-Type: application/json\r\n\r\n{}\r\n--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{\"test\":1}}}\r\n--graphql--\r\n",
            ),
        ))];

        let body = StreamBody::new(futures::stream::iter(chunks));
        let mut stream = parse_to_stream("graphql", body);

        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let response = first_result.unwrap();
        assert!(!response.data.is_null());

        let second = stream.next().await;
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn test_parse_to_stream_transport_error() {
        use futures::StreamExt;
        use http_body_util::StreamBody;
        use hyper::body::Frame;

        let chunks: Vec<Result<Frame<Bytes>, std::convert::Infallible>> = vec![Ok(Frame::data(
            Bytes::from(
                "--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":null,\"errors\":[{\"message\":\"Connection lost\"}]}\r\n--graphql--\r\n",
            ),
        ))];

        let body = StreamBody::new(futures::stream::iter(chunks));
        let mut stream = parse_to_stream("graphql", body);

        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let response = first_result.unwrap();
        assert!(response.errors.is_some());
        let errors = response.errors.unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Connection lost");

        let second = stream.next().await;
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn test_parse_to_stream_custom_boundary() {
        use futures::StreamExt;
        use http_body_util::StreamBody;
        use hyper::body::Frame;

        let chunks: Vec<Result<Frame<Bytes>, std::convert::Infallible>> = vec![Ok(Frame::data(
            Bytes::from(
                "--myboundary\r\nContent-Type: application/json\r\n\r\n{\"data\":{\"test\":1}}\r\n--myboundary--\r\n",
            ),
        ))];

        let body = StreamBody::new(futures::stream::iter(chunks));
        let mut stream = parse_to_stream("myboundary", body);

        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let response = first_result.unwrap();
        assert!(!response.data.is_null());

        let second = stream.next().await;
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn test_parse_to_stream_without_payload_wrapper() {
        use futures::StreamExt;
        use http_body_util::StreamBody;
        use hyper::body::Frame;

        let chunks: Vec<Result<Frame<Bytes>, std::convert::Infallible>> = vec![Ok(Frame::data(
            Bytes::from(
                "--boundary\r\nContent-Type: application/json\r\n\r\n{\"data\":{\"user\":\"Alice\"}}\r\n--boundary--\r\n",
            ),
        ))];

        let body = StreamBody::new(futures::stream::iter(chunks));
        let mut stream = parse_to_stream("boundary", body);

        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let response = first_result.unwrap();
        assert!(!response.data.is_null());

        let second = stream.next().await;
        assert!(second.is_none());
    }
}
