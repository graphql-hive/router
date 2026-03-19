use bytes::{Buf, Bytes};
use futures::stream::BoxStream;
use http_body_util::BodyExt;
use hyper::body::Body;

use crate::{
    executors::error::SubgraphExecutorError, response::subgraph_response::SubgraphResponse,
};

#[derive(thiserror::Error, Debug)]
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
            while let Some(boundary) = find_sse_event_boundary(&buffer) {
                let event_bytes: Vec<u8> = buffer.drain(..boundary).collect();

                match parse(&event_bytes) {
                    Ok(Some(sse_event)) => {
                        match sse_event.event.as_deref() {
                            Some("next") if !sse_event.data.is_empty() => {
                                match SubgraphResponse::deserialize_from_bytes(Bytes::from(sse_event.data.clone())) {
                                    Ok(response) => {
                                        yield Ok(response);
                                    }
                                    Err(e) => {
                                        yield Err(ParseError::InvalidSubgraphResponse(e));
                                        return;
                                    }
                                }
                            }
                            Some("complete") => {
                                return;
                            }
                            _ => {
                                // ping
                            }
                        }
                    }
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                    _ => {}
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

#[derive(Debug)]
struct SubgraphSseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// find the boundary of a complete event in the stream, the '\n\n'
fn find_sse_event_boundary(buffer: &[u8]) -> Option<usize> {
    for i in 0..buffer.len().saturating_sub(1) {
        if buffer[i] == b'\n' && buffer[i + 1] == b'\n' {
            return Some(i + 2);
        }
    }
    None
}

// returns at most one event because the caller always passes exactly one event's bytes
// (drained up to the \n\n boundary by find_sse_event_boundary)
fn parse(raw: &[u8]) -> Result<Option<SubgraphSseEvent>, ParseError> {
    let text = std::str::from_utf8(raw).map_err(|e| ParseError::InvalidUtf8(e.to_string()))?;

    let mut current_event: Option<String> = None;
    let mut current_data_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        if line.is_empty() {
            if current_event.is_some() || !current_data_lines.is_empty() {
                return Ok(Some(SubgraphSseEvent {
                    event: current_event,
                    data: current_data_lines.join("\n"),
                }));
            }
            continue;
        }

        if line.starts_with(':') {
            // heartbeat
            continue;
        }

        if let Some(colon_pos) = line.find(':') {
            let field = &line[..colon_pos];
            let value = &line[colon_pos + 1..];

            let value = value.trim();

            match field {
                "event" => {
                    current_event = Some(value.to_string());
                }
                "data" => {
                    current_data_lines.push(value.to_string());
                }
                _ => {
                    // ignore unknown fields as per SSE spec
                }
            }
        }
    }

    // handle any remaining event that wasn't terminated with empty line(s)
    if current_event.is_some() || !current_data_lines.is_empty() {
        return Ok(Some(SubgraphSseEvent {
            event: current_event,
            data: current_data_lines.join("\n"),
        }));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_event_with_data() {
        let sse_data = br#"event: next
data: some data

"#;

        let event = parse(sse_data).expect("Should parse valid SSE");

        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.event, Some("next".to_string()));
        assert_eq!(event.data, "some data");
    }

    #[test]
    fn test_parse_event_without_explicit_type() {
        let sse_data = b"data: some data\n\n";

        let event = parse(sse_data).expect("Should parse valid SSE");

        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.event, None);
        assert_eq!(event.data, "some data");
    }

    #[test]
    fn test_parse_just_event() {
        let sse_data = b"event: complete\n\n";

        let event = parse(sse_data).expect("Should parse valid SSE");

        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.event, Some("complete".to_string()));
        assert_eq!(event.data, "");
    }

    #[test]
    fn test_parse_multiple_events() {
        let sse_data = br#"event: next
data: value 1

event: next
data: value 2

event: complete

"#;

        let event = parse(sse_data).expect("Should parse valid SSE");

        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.event, Some("next".to_string()));
        assert_eq!(event.data, "value 1");
    }

    #[test]
    fn test_parse_multiline_data() {
        // SSE spec allows data to be split across multiple "data:" lines
        // even though we'll not often see this in practice, good to cover
        let sse_data = b"event: next\ndata: line1\ndata: line2\n\n";

        let event = parse(sse_data).expect("Should parse valid SSE");

        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.event, Some("next".to_string()));
        assert_eq!(event.data, "line1\nline2");
    }

    #[test]
    fn test_parse_no_double_newline() {
        // even though we'll never see this in practice
        let sse_data = b"event: next\ndata: line0";

        let event = parse(sse_data).expect("Should parse valid SSE");

        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.event, Some("next".to_string()));
        assert_eq!(event.data, "line0");
    }

    #[test]
    fn test_parse_heartbeat() {
        let sse_data = b":\n\nevent: next\ndata: payload\n\n:\n\n";

        let event = parse(sse_data).expect("Should parse valid SSE");

        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.event, Some("next".to_string()));
        assert_eq!(event.data, "payload");
    }

    #[test]
    fn test_parse_empty_input() {
        let sse_data = b"";

        let event = parse(sse_data).expect("Should handle empty input");

        assert!(event.is_none());
    }

    #[tokio::test]
    async fn test_parse_to_stream_chunked_events() {
        use bytes::Bytes;
        use futures::StreamExt;
        use http_body_util::StreamBody;
        use hyper::body::Frame;

        // chunked delivery where events are split across multiple chunks
        let chunks: Vec<Result<Frame<Bytes>, std::convert::Infallible>> = vec![
            Ok(Frame::data(Bytes::from(
                "event: next\ndata: {\"data\":{\"hello\":\"wor",
            ))),
            Ok(Frame::data(Bytes::from("ld\"}}\n\neve"))),
            Ok(Frame::data(Bytes::from("nt: complete\n\n"))),
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
        assert!(second.is_none());
    }
}
