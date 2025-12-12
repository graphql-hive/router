#[derive(Debug, Clone, PartialEq)]
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

#[derive(thiserror::Error, Debug, Clone)]
enum SseParseError {
    #[error("Invalid UTF-8 sequence: {0}")]
    InvalidUFT8Sequence(std::str::Utf8Error),
}

fn parse(raw: &[u8]) -> Result<Vec<SubgraphSseEvent>, SseParseError> {
    let text = std::str::from_utf8(raw).map_err(|e| SseParseError::InvalidUFT8Sequence(e))?;

    let mut events = Vec::new();
    let mut current_event: Option<String> = None;
    let mut current_data_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        if line.is_empty() {
            if current_event.is_some() || !current_data_lines.is_empty() {
                // if the data lines are not empty, this is the second new line - event boundary
                events.push(SubgraphSseEvent {
                    event: current_event.clone(),
                    data: current_data_lines.join("\n"),
                });
                current_event = None;
                current_data_lines.clear();
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
        let data = current_data_lines.join("\n");
        events.push(SubgraphSseEvent {
            event: current_event,
            data,
        });
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_event_with_data() {
        let sse_data = br#"event: next
data: some data

"#;

        let events = parse(sse_data).expect("Should parse valid SSE");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, Some("next".to_string()));
        assert_eq!(events[0].data, "some data");
    }

    #[test]
    fn test_parse_event_without_explicit_type() {
        let sse_data = b"data: some data\n\n";

        let events = parse(sse_data).expect("Should parse valid SSE");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, None);
        assert_eq!(events[0].data, "some data");
    }

    #[test]
    fn test_parse_just_event() {
        let sse_data = b"event: complete\n\n";

        let events = parse(sse_data).expect("Should parse valid SSE");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, Some("complete".to_string()));
        assert_eq!(events[0].data, "");
    }

    #[test]
    fn test_parse_multiple_events() {
        let sse_data = br#"event: next
data: value 1

event: next
data: value 2

event: complete

"#;

        let events = parse(sse_data).expect("Should parse valid SSE");

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event, Some("next".to_string()));
        assert_eq!(events[0].data, "value 1");

        assert_eq!(events[1].event, Some("next".to_string()));
        assert_eq!(events[1].data, "value 2");

        assert_eq!(events[2].event, Some("complete".to_string()));
        assert_eq!(events[2].data, "");
    }

    #[test]
    fn test_parse_multiline_data() {
        // SSE spec allows data to be split across multiple "data:" lines
        // even though we'll not often see this in practice, good to cover
        let sse_data = b"event: next\ndata: line1\ndata: line2\n\n";

        let events = parse(sse_data).expect("Should parse valid SSE");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, Some("next".to_string()));
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn test_parse_no_double_newline() {
        // even though we'll never see this in practice
        let sse_data = b"event: next\ndata: line0";

        let events = parse(sse_data).expect("Should parse valid SSE");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, Some("next".to_string()));
        assert_eq!(events[0].data, "line0");
    }

    #[test]
    fn test_parse_heartbeat() {
        let sse_data = b":\n\nevent: next\ndata: payload\n\n:\n\n";

        let events = parse(sse_data).expect("Should parse valid SSE");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, Some("next".to_string()));
        assert_eq!(events[0].data, "payload");
    }

    #[test]
    fn test_parse_empty_input() {
        let sse_data = b"";

        let events = parse(sse_data).expect("Should handle empty input");

        assert_eq!(events.len(), 0);
    }
}
