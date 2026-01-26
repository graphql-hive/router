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
    let buffer_str = std::str::from_utf8(buffer).ok()?;

    if !started {
        let pos = buffer_str.find(delimiter)?;
        let after_delimiter = pos + delimiter.len();
        let newline_pos = buffer_str[after_delimiter..].find('\n')?;
        let skip_len = after_delimiter + newline_pos + 1;
        return Some((0, skip_len, false));
    }

    let next_delimiter_pos = buffer_str.find(delimiter)?;
    let is_end = buffer_str[next_delimiter_pos..].starts_with(end_marker);

    let skip_len = if is_end {
        let after_end = next_delimiter_pos + end_marker.len();
        if let Some(newline) = buffer_str[after_end..].find('\n') {
            end_marker.len() + newline + 1
        } else {
            end_marker.len()
        }
    } else {
        let after_delimiter = next_delimiter_pos + delimiter.len();
        if let Some(newline) = buffer_str[after_delimiter..].find('\n') {
            delimiter.len() + newline + 1
        } else {
            delimiter.len()
        }
    };

    Some((next_delimiter_pos, skip_len, is_end))
}

fn parse_part(raw: &[u8]) -> Result<Option<SubgraphResponse<'static>>, ParseError> {
    let text = std::str::from_utf8(raw).map_err(|e| ParseError::InvalidUtf8(e.to_string()))?;
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
