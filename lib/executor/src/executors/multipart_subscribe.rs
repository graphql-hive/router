use bytes::{Buf, Bytes};
use futures::stream::BoxStream;
use http_body_util::BodyExt;
use hyper::body::Body;

/// The boundary used in multipart subscription responses.
/// Per the spec, this is always "graphql".
const BOUNDARY_MARKER: &str = "--graphql";
const END_MARKER: &str = "--graphql--";

#[derive(thiserror::Error, Debug, Clone)]
pub enum ParseError {
    #[error("Invalid UTF-8 sequence: {0}")]
    InvalidUtf8(String),
    #[error("Stream read error: {0}")]
    StreamReadError(String),
    #[error("Missing 'payload' field in multipart response")]
    MissingPayload,
    #[error("Invalid JSON: {0}")]
    InvalidJson(String),
}

pub fn parse_to_stream<B>(body_stream: B) -> BoxStream<'static, Result<Bytes, ParseError>>
where
    B: Body + Send + Unpin + 'static,
    B::Data: Buf + Send,
    B::Error: std::fmt::Display + Send,
{
    let stream = async_stream::stream! {
        let mut body = body_stream;
        let mut buffer = Vec::<u8>::new();

        loop {
            while let Some((boundary_end, is_end)) = find_multipart_boundary(&buffer) {
                let part_bytes: Vec<u8> = buffer.drain(..boundary_end).collect();

                if !part_bytes.is_empty() {
                    match parse_part(&part_bytes) {
                        Ok(Some(payload)) => {
                            yield Ok(Bytes::from(payload));
                        }
                        Ok(None) => {
                            // heartbeat, skip
                        }
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

/// Find the boundary of a complete multipart part in the buffer.
/// Returns the index just past the part (up to the next boundary), and whether it's the end marker.
///
/// Multipart format per multipart-spec.md:
/// ```text
/// --graphql
/// Content-Type: application/json
///
/// {"payload":{"data":{...}}}
/// --graphql
/// ...
/// --graphql--
/// ```
fn find_multipart_boundary(buffer: &[u8]) -> Option<(usize, bool)> {
    let buffer_str = std::str::from_utf8(buffer).ok()?;

    // Find the first boundary marker
    let first_boundary_pos = buffer_str.find(BOUNDARY_MARKER)?;

    // Look for the next boundary marker after the first one
    let after_first = first_boundary_pos + BOUNDARY_MARKER.len();
    if after_first >= buffer_str.len() {
        return None;
    }

    // Find the next boundary
    if let Some(next_pos) = buffer_str[after_first..].find(BOUNDARY_MARKER) {
        let absolute_next_pos = after_first + next_pos;

        // Check if this is the end marker
        let is_end = buffer_str[absolute_next_pos..].starts_with(END_MARKER);

        return Some((absolute_next_pos, is_end));
    }

    None
}

/// Parse a single multipart part and extract the GraphQL payload.
/// Returns None for heartbeats (empty object `{}`).
fn parse_part(raw: &[u8]) -> Result<Option<String>, ParseError> {
    let text = std::str::from_utf8(raw).map_err(|e| ParseError::InvalidUtf8(e.to_string()))?;

    // Skip the boundary marker and its trailing newline
    let content = if let Some(pos) = text.find(BOUNDARY_MARKER) {
        let after_boundary = &text[pos + BOUNDARY_MARKER.len()..];
        after_boundary
            .trim_start_matches("\r\n")
            .trim_start_matches('\n')
    } else {
        text
    };

    // Split headers from body (separated by double CRLF or double LF)
    let body = if let Some(pos) = content.find("\r\n\r\n") {
        content[pos + 4..].trim()
    } else if let Some(pos) = content.find("\n\n") {
        content[pos + 2..].trim()
    } else {
        // No headers, entire content is body
        content.trim()
    };

    if body.is_empty() {
        return Ok(None);
    }

    extract_payload(body)
}

/// Extract the GraphQL payload from a multipart part body.
///
/// Per multipart-spec.md:
/// - Regular events have: `{"payload": {"data": {...}}}`
/// - Heartbeats are: `{}` (empty object, no payload wrapper)
/// - Transport errors have: `{"payload": null, "errors": [...]}`
///
/// Returns the inner payload body (the GraphQL response), or None for heartbeats.
fn extract_payload(body: &str) -> Result<Option<String>, ParseError> {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    // Parse the body as JSON
    let parsed: sonic_rs::Value =
        sonic_rs::from_str(body).map_err(|e| ParseError::InvalidJson(e.to_string()))?;

    // Check if this is a heartbeat (empty object)
    if parsed.is_object() {
        let obj = parsed.as_object().unwrap();
        if obj.is_empty() {
            return Ok(None); // Heartbeat
        }
    }

    // Check for transport-level error (payload is null, errors at top level)
    if parsed.get("payload").map(|v| v.is_null()).unwrap_or(false) {
        if let Some(errors) = parsed.get("errors") {
            // Return the transport error as a GraphQL error response
            let error_response = format!(
                r#"{{"errors":{}}}"#,
                sonic_rs::to_string(errors).unwrap_or_default()
            );
            return Ok(Some(error_response));
        }
    }

    // Extract the payload field
    if let Some(payload) = parsed.get("payload") {
        if payload.is_null() {
            return Ok(None);
        }
        // Return the payload content as string (the GraphQL response)
        return Ok(Some(
            sonic_rs::to_string(payload).map_err(|e| ParseError::InvalidJson(e.to_string()))?,
        ));
    }

    // If there's no payload field, it's an invalid format
    Err(ParseError::MissingPayload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_part_with_headers() {
        let part_data =
            b"--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{\"reviewAdded\":{\"id\":\"1\"}}}}";

        let payload = parse_part(part_data)
            .expect("Should parse valid part")
            .expect("Should have payload");

        assert!(payload.contains("reviewAdded"));
        assert!(payload.contains("id"));
    }

    #[test]
    fn test_parse_part_without_headers() {
        let part_data = b"--graphql\r\n\r\n{\"payload\":{\"data\":{\"value\":1}}}";

        let payload = parse_part(part_data)
            .expect("Should parse part without headers")
            .expect("Should have payload");

        assert!(payload.contains("value"));
    }

    #[test]
    fn test_extract_payload_normal() {
        let body = r#"{"payload":{"data":{"reviewAdded":{"id":"1"}}}}"#;

        let payload = extract_payload(body)
            .expect("Should extract payload")
            .expect("Should have payload");

        assert!(payload.contains("reviewAdded"));
        assert!(payload.contains("id"));
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

        let payload = extract_payload(body)
            .expect("Should extract error")
            .expect("Should have error response");

        assert!(payload.contains("errors"));
        assert!(payload.contains("Connection lost"));
    }

    #[test]
    fn test_find_multipart_boundary() {
        let buffer =
            b"--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{}}}\r\n--graphql\r\n";

        let result = find_multipart_boundary(buffer);

        assert!(result.is_some());
        let (pos, is_end) = result.unwrap();
        assert!(!is_end);
        assert!(pos > 0);
    }

    #[test]
    fn test_find_multipart_boundary_end_marker() {
        let buffer =
            b"--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{}}}\r\n--graphql--\r\n";

        let result = find_multipart_boundary(buffer);

        assert!(result.is_some());
        let (_, is_end) = result.unwrap();
        assert!(is_end);
    }

    #[test]
    fn test_find_multipart_boundary_incomplete() {
        let buffer = b"--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{}}}";

        let result = find_multipart_boundary(buffer);

        assert!(result.is_none());
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
        let mut stream = parse_to_stream(body);

        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let payload = first_result.unwrap();
        assert!(std::str::from_utf8(&payload).unwrap().contains("test"));

        let second = stream.next().await;
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn test_parse_to_stream_chunked_events() {
        use futures::StreamExt;
        use http_body_util::StreamBody;
        use hyper::body::Frame;

        // Chunked delivery where events are split across multiple chunks
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
        let mut stream = parse_to_stream(body);

        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let payload = String::from_utf8(first_result.unwrap().to_vec()).unwrap();
        assert!(payload.contains("value"));
        assert!(payload.contains("1"));

        let second = stream.next().await;
        assert!(second.is_some());
        let second_result = second.unwrap();
        assert!(second_result.is_ok());
        let payload = String::from_utf8(second_result.unwrap().to_vec()).unwrap();
        assert!(payload.contains("value"));
        assert!(payload.contains("2"));

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
        let mut stream = parse_to_stream(body);

        // First event should be the data (heartbeat is skipped)
        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let payload = String::from_utf8(first_result.unwrap().to_vec()).unwrap();
        assert!(payload.contains("test"));

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
        let mut stream = parse_to_stream(body);

        let first = stream.next().await;
        assert!(first.is_some());
        let first_result = first.unwrap();
        assert!(first_result.is_ok());
        let payload = String::from_utf8(first_result.unwrap().to_vec()).unwrap();
        assert!(payload.contains("errors"));
        assert!(payload.contains("Connection lost"));

        let second = stream.next().await;
        assert!(second.is_none());
    }
}
