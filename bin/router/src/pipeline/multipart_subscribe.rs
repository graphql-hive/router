// TODO: test thoroughly

use const_str::concat;

use futures_util::{Stream, StreamExt};
use ntex::util::{Bytes, BytesMut};
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
macro_rules! make_delimiters {
    ($boundary:expr) => {
        (
            // opens the stream
            concat!("--", $boundary),
            // closes a part. it is written together with the part, not before the next one, because
            // clients hand a part on only once they see the delimiter that closes it
            concat!("\r\n--", $boundary),
        )
    };
}

// the line break ends the delimiter before the part
const PART_HEADERS: &str = "\r\nContent-Type: application/json\r\n\r\n";

// follows the delimiter that closed the last part
const CLOSE_DELIMITER_SUFFIX: &str = "--";

/// One part with its headers and the delimiter that closes it, in a single chunk.
fn make_part(body: &[&[u8]], part_delimiter: &str) -> Bytes {
    let body_len: usize = body.iter().map(|b| b.len()).sum();
    let mut part = BytesMut::with_capacity(PART_HEADERS.len() + body_len + part_delimiter.len());
    part.put_slice(PART_HEADERS.as_bytes());
    for chunk in body {
        part.put_slice(chunk);
    }
    part.put_slice(part_delimiter.as_bytes());
    part.freeze()
}

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
    let mut input = input;
    let (open_delimiter, part_delimiter) = make_delimiters!(INCREMENTAL_DELIVERY_BOUNDARY);
    async_stream::stream! {
        yield Ok(Bytes::from(open_delimiter));
        loop {
            match input.next().await {
                Some(resp) => {
                    match std::str::from_utf8(&resp) {
                        Ok(_) => {
                            yield Ok(make_part(&[&resp], part_delimiter));
                        }
                        Err(e) => {
                            yield Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                            break;
                        }
                    }
                }
                None => {
                    yield Ok(Bytes::from(CLOSE_DELIMITER_SUFFIX));
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
    let mut input = input;
    let (open_delimiter, part_delimiter) = make_delimiters!(APOLLO_MULTIPART_HTTP_BOUNDARY);
    async_stream::stream! {
        yield Ok(Bytes::from(open_delimiter));
        loop {
            tokio::select! {
                item = input.next() => {
                    match item {
                        Some(resp) => {
                            match std::str::from_utf8(&resp) {
                                Ok(_) => {
                                    // Wrap the GraphQL response in a payload field
                                    // As per the spec.
                                    yield Ok(make_part(&[br#"{"payload":"#, &resp, b"}"], part_delimiter));
                                }
                                Err(e) => {
                                    // TODO: use transport level errors as per spec
                                    yield Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                                    break;
                                }
                            }
                        }
                        None => {
                            yield Ok(Bytes::from(CLOSE_DELIMITER_SUFFIX));
                            break;
                        },
                    }
                }
                _ = tokio::time::sleep(heartbeat_interval) => {
                    yield Ok(make_part(&[b"{}"], part_delimiter));
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

    #[tokio::test]
    async fn apollo_multipart_closes_a_heartbeat_before_the_next_part_arrives() {
        let mut output =
            create_apollo_multipart_http_stream(stream::pending(), Duration::from_millis(1));

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
