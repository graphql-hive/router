use bytes::{Buf, Bytes};
use futures::stream::BoxStream;
use http_body_util::BodyExt;
use hyper::body::Body;

use crate::{
    executors::error::SubgraphExecutorError, response::subgraph_response::SubgraphResponse,
};

#[derive(thiserror::Error, Debug, Clone)]
pub enum ParseError {
    #[error("Invalid UTF-8 sequence: {0}")]
    InvalidUtf8(String),
    #[error("Stream read error: {0}")]
    StreamReadError(String),
    #[error("Invalid subgraph response: {0}")]
    InvalidSubgraphResponse(SubgraphExecutorError),
}

pub fn parse_to_stream<B>(
    body_stream: B,
) -> BoxStream<'static, Result<SubgraphResponse<'static>, ParseError>>
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

fn find_multipart_boundary(buffer: &[u8]) -> Option<(usize, bool)> {
    let buffer_str = std::str::from_utf8(buffer).ok()?;

    let first_boundary_pos = find_first_boundary(buffer_str)?;
    let boundary_marker = extract_boundary_marker(buffer_str, first_boundary_pos)?;

    let after_first = first_boundary_pos + boundary_marker.len();
    if after_first >= buffer_str.len() {
        return None;
    }

    if let Some(next_pos) = buffer_str[after_first..].find(&boundary_marker) {
        let absolute_next_pos = after_first + next_pos;
        let end_marker = format!("{}--", boundary_marker);
        let is_end = buffer_str[absolute_next_pos..].starts_with(&end_marker);
        return Some((absolute_next_pos, is_end));
    }

    None
}

fn find_first_boundary(text: &str) -> Option<usize> {
    text.find("--")
}

fn extract_boundary_marker(text: &str, start: usize) -> Option<String> {
    let after_dashes = &text[start..];
    let boundary_end = after_dashes
        .find('\r')
        .or_else(|| after_dashes.find('\n'))?;

    let boundary = &after_dashes[..boundary_end];
    Some(boundary.to_string())
}

fn parse_part(raw: &[u8]) -> Result<Option<SubgraphResponse<'static>>, ParseError> {
    let text = std::str::from_utf8(raw).map_err(|e| ParseError::InvalidUtf8(e.to_string()))?;

    let content = skip_boundary_line(text);
    let body = extract_body_after_headers(content);

    if body.is_empty() {
        return Ok(None);
    }

    extract_payload(body)
}

fn skip_boundary_line(text: &str) -> &str {
    if let Some(pos) = text.find("--") {
        let after_boundary = &text[pos..];
        if let Some(newline_pos) = after_boundary.find('\n') {
            return &after_boundary[newline_pos + 1..];
        }
    }
    text
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
    use sonic_rs::JsonValueTrait;

    let parsed: sonic_rs::Value = sonic_rs::from_str(body).map_err(|e| {
        ParseError::InvalidSubgraphResponse(SubgraphExecutorError::ResponseDeserializationFailure(
            e.to_string(),
        ))
    })?;

    if is_heartbeat(&parsed) {
        return Ok(None);
    }

    if let Some(transport_error) = extract_transport_error(&parsed) {
        return SubgraphResponse::deserialize_from_bytes(Bytes::from(transport_error))
            .map_err(ParseError::InvalidSubgraphResponse)
            .map(Some);
    }

    if let Some(payload) = parsed.get("payload") {
        if payload.is_null() {
            return Ok(None);
        }
        let payload_str = sonic_rs::to_string(payload).map_err(|e| {
            ParseError::InvalidSubgraphResponse(
                SubgraphExecutorError::ResponseDeserializationFailure(e.to_string()),
            )
        })?;
        return SubgraphResponse::deserialize_from_bytes(Bytes::from(payload_str))
            .map_err(ParseError::InvalidSubgraphResponse)
            .map(Some);
    }

    SubgraphResponse::deserialize_from_bytes(Bytes::from(body.to_owned()))
        .map_err(ParseError::InvalidSubgraphResponse)
        .map(Some)
}

fn is_heartbeat(value: &sonic_rs::Value) -> bool {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    value.is_object() && value.as_object().is_some_and(|obj| obj.is_empty())
}

fn extract_transport_error(value: &sonic_rs::Value) -> Option<String> {
    use sonic_rs::JsonValueTrait;

    if value.get("payload").map(|v| v.is_null()).unwrap_or(false) {
        if let Some(errors) = value.get("errors") {
            return Some(format!(
                r#"{{"errors":{}}}"#,
                sonic_rs::to_string(errors).unwrap_or_default()
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_parse_part_with_headers() {
        let part_data =
            b"--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{\"reviewAdded\":{\"id\":\"1\"}}}}";

        let response = parse_part(part_data)
            .expect("Should parse valid part")
            .expect("Should have response");

        assert!(!response.data.is_null());
    }

    #[test]
    fn test_parse_part_without_headers() {
        let part_data = b"--graphql\r\n\r\n{\"payload\":{\"data\":{\"value\":1}}}";

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

    #[test]
    fn test_find_multipart_boundary_graphql() {
        let buffer =
            b"--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{}}}\r\n--graphql\r\n";

        let result = find_multipart_boundary(buffer);

        assert!(result.is_some());
        let (pos, is_end) = result.unwrap();
        assert!(!is_end);
        assert!(pos > 0);
    }

    #[test]
    fn test_find_multipart_boundary_custom() {
        let buffer =
            b"--myboundary\r\nContent-Type: application/json\r\n\r\n{\"data\":{}}\r\n--myboundary\r\n";

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
    fn test_find_multipart_boundary_custom_end_marker() {
        let buffer =
            b"--xyz123\r\nContent-Type: application/json\r\n\r\n{\"data\":{}}\r\n--xyz123--\r\n";

        let result = find_multipart_boundary(buffer);

        assert!(result.is_some());
        let (_, is_end) = result.unwrap();
        assert!(is_end);
    }

    #[test]
    fn test_find_multipart_boundary_incomplete() {
        let buffer =
            b"--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{}}}";

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
        let mut stream = parse_to_stream(body);

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
        let mut stream = parse_to_stream(body);

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
        let mut stream = parse_to_stream(body);

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
        let mut stream = parse_to_stream(body);

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
        let mut stream = parse_to_stream(body);

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
