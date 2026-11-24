// TODO: test thoroughly

use const_str::concat;

use futures_util::{Stream, StreamExt};
use ntex::util::Bytes;
use std::time::Duration;
use tokio_util::bytes::BufMut;

// we use macros to retain constness
macro_rules! make_content_type {
    ($boundary:expr) => {
        // wrapping with "" is not necessary in our case since the boundaries we use do not contain special
        // characters - but clients out there probably rely on the quotes so we add them just in case
        concat!("multipart/mixed;boundary=\"", $boundary, "\"")
    };
}

// the line break ends the delimiter before the part
const PART_HEADERS: &str = "\r\nContent-Type: application/json\r\n\r\n";

const INCREMENTAL_DELIVERY_BOUNDARY: &str = "-";

pub const INCREMENTAL_DELIVERY_CONTENT_TYPE: &str =
    make_content_type!(INCREMENTAL_DELIVERY_BOUNDARY);

/// Create a multipart subscription stream following the Official GraphQL over HTTP Incremental Delivery RFC.
///
/// Will use `-` as boundary.
///
/// NOTE: Incremental Delivery over HTTP does not support heartbeats. Please prefer Apollo's multiple HTTP where applicable.
///
/// Read more: https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol
pub fn create_incremental_delivery_stream(
    input: impl Stream<Item = Vec<u8>> + Send + Unpin + 'static,
) -> impl Stream<Item = Result<ntex::util::Bytes, std::io::Error>> + Unpin {
    const OPEN_DELIMITER: &str = concat!("--", INCREMENTAL_DELIVERY_BOUNDARY);
    // closes a part. written right after the part, not before the next one, because
    // clients hand a part on only once they see the delimiter that closes it
    const PART_DELIMITER: &str = concat!("\r\n--", INCREMENTAL_DELIVERY_BOUNDARY);
    let mut input = input;
    async_stream::stream! {
        yield Ok(Bytes::from(OPEN_DELIMITER));
        loop {
            match input.next().await {
                Some(resp) => {
                    match std::str::from_utf8(&resp) {
                        Ok(_) => {
                            // separate chunks so resp isnt copied, the delimiter still goes out right after the part
                            yield Ok(Bytes::from(PART_HEADERS));
                            yield Ok(Bytes::from(resp));
                            yield Ok(Bytes::from(PART_DELIMITER));
                        }
                        Err(e) => {
                            yield Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                            break;
                        }
                    }
                }
                None => {
                    // turns the last part delimiter into the close delimiter
                    yield Ok(Bytes::from("--"));
                    break;
                },
            }
        }
    }
    .boxed()
}

const APOLLO_MULTIPART_HTTP_BOUNDARY: &str = "graphql";

pub const APOLLO_MULTIPART_HTTP_CONTENT_TYPE: &str = concat!(
    make_content_type!(APOLLO_MULTIPART_HTTP_BOUNDARY),
    ";subscriptionSpec=1.0"
);

/// Create a multipart subscription stream following Apollo's Multipart HTTP spec.
///
/// Will use `graphql` as boundary.
///
/// Read more: https://github.com/graphql/graphql-over-http/blob/d312e43384006fa323b918d49cfd9fbd76ac1257/rfcs/IncrementalDelivery.md
pub fn create_apollo_multipart_http_stream(
    input: impl Stream<Item = Vec<u8>> + Send + Unpin + 'static,
    heartbeat_interval: Duration,
) -> impl Stream<Item = Result<ntex::util::Bytes, std::io::Error>> + Unpin {
    const OPEN_DELIMITER: &str = concat!("--", APOLLO_MULTIPART_HTTP_BOUNDARY);
    // see create_incremental_delivery_stream
    const PART_DELIMITER: &str = concat!("\r\n--", APOLLO_MULTIPART_HTTP_BOUNDARY);
    const HEARTBEAT: &str = concat!(PART_HEADERS, "{}", PART_DELIMITER);
    let mut input = input;
    async_stream::stream! {
        yield Ok(Bytes::from(OPEN_DELIMITER));
        loop {
            tokio::select! {
                item = input.next() => {
                    match item {
                        Some(resp) => {
                            match std::str::from_utf8(&resp) {
                                Ok(_) => {
                                    // Wrap the GraphQL response in a payload field
                                    // As per the spec.
                                    let mut part = ntex::util::BytesMut::with_capacity(
                                        PART_HEADERS.len() + resp.len() + 13 + PART_DELIMITER.len(),
                                    );
                                    part.put_slice(PART_HEADERS.as_bytes());
                                    part.put_slice(br#"{"payload":"#);
                                    part.put_slice(&resp);
                                    part.put_slice(b"}");
                                    part.put_slice(PART_DELIMITER.as_bytes());
                                    yield Ok(part.freeze());
                                }
                                Err(e) => {
                                    // TODO: use transport level errors as per spec
                                    yield Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                                    break;
                                }
                            }
                        }
                        None => {
                            yield Ok(Bytes::from("--"));
                            break;
                        },
                    }
                }
                _ = tokio::time::sleep(heartbeat_interval) => {
                    yield Ok(Bytes::from(HEARTBEAT));
                }
            }
        }
    }.boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{stream, FutureExt};

    /// Everything the stream has ready, without waiting for more input.
    fn read_ready(
        output: &mut (impl Stream<Item = Result<Bytes, std::io::Error>> + Unpin),
    ) -> String {
        let mut read = String::new();
        while let Some(Some(chunk)) = output.next().now_or_never() {
            read.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
        }
        read
    }

    #[tokio::test]
    async fn apollo_multipart_closes_a_part_before_the_next_one_arrives() {
        let input = stream::iter([br#"{"data":{"a":1}}"#.to_vec()]).chain(stream::pending());
        let mut output = create_apollo_multipart_http_stream(input, Duration::from_secs(3600));

        assert_eq!(
            read_ready(&mut output),
            "--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{\"a\":1}}}\r\n--graphql"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn apollo_multipart_closes_a_heartbeat_before_the_next_part_arrives() {
        let mut output =
            create_apollo_multipart_http_stream(stream::pending(), Duration::from_secs(10));

        assert_eq!(read_ready(&mut output), "--graphql");
        assert_eq!(
            std::str::from_utf8(&output.next().await.unwrap().unwrap()).unwrap(),
            "\r\nContent-Type: application/json\r\n\r\n{}\r\n--graphql"
        );
    }

    #[tokio::test]
    async fn incremental_delivery_closes_a_part_before_the_next_one_arrives() {
        let input = stream::iter([br#"{"data":{"a":1}}"#.to_vec()]).chain(stream::pending());
        let mut output = create_incremental_delivery_stream(input);

        assert_eq!(
            read_ready(&mut output),
            "---\r\nContent-Type: application/json\r\n\r\n{\"data\":{\"a\":1}}\r\n---"
        );
    }

    #[tokio::test]
    async fn ended_streams_are_framed_as_before() {
        let input = stream::iter([b"{}".to_vec(), b"{}".to_vec()]);
        let output = create_incremental_delivery_stream(input);
        let read: Vec<u8> = output.map(|chunk| chunk.unwrap().to_vec()).concat().await;

        assert_eq!(
            std::str::from_utf8(&read).unwrap(),
            "---\r\nContent-Type: application/json\r\n\r\n{}\r\n---\r\nContent-Type: application/json\r\n\r\n{}\r\n-----"
        );
    }
}
