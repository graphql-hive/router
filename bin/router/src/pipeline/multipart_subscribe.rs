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

/// What comes between a boundary and a part's payload.
const PART_HEAD: &str = "\r\nContent-Type: application/json\r\n\r\n";
/// After a boundary, this makes it the closing one.
const CLOSE: &str = "--";

const INCREMENTAL_DELIVERY_BOUNDARY: &str = "-";

pub const INCREMENTAL_DELIVERY_CONTENT_TYPE: &str =
    make_content_type!(INCREMENTAL_DELIVERY_BOUNDARY);

/// `@defer` responses say which format the parts are in.
pub const INCREMENTAL_DELIVERY_DEFER_CONTENT_TYPE: &str = concat!(
    make_content_type!(INCREMENTAL_DELIVERY_BOUNDARY),
    ";incrementalSpec=v0.2"
);
pub const INCREMENTAL_DELIVERY_LEGACY_DEFER_CONTENT_TYPE: &str = concat!(
    make_content_type!(INCREMENTAL_DELIVERY_BOUNDARY),
    ";deferSpec=20220824"
);

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
    // Every part goes out with the boundary that ends it. A client only knows a part is whole
    // once it sees that boundary, so with the boundary in front of the next part it would sit
    // on each part until the next one came. The bytes are the same either way.
    const BOUNDARY: &str = concat!("--", INCREMENTAL_DELIVERY_BOUNDARY);
    const PART_END: &str = concat!("\r\n", BOUNDARY);
    async_stream::stream! {
        yield Ok(Bytes::from(BOUNDARY));
        loop {
            match input.next().await {
                Some(resp) => {
                    match std::str::from_utf8(&resp) {
                        Ok(_) => {
                            yield Ok(Bytes::from(PART_HEAD));
                            yield Ok(Bytes::from(resp));
                            yield Ok(Bytes::from(PART_END));
                        }
                        Err(e) => {
                            yield Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                            break;
                        }
                    }
                }
                None => {
                    yield Ok(Bytes::from(CLOSE));
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
    // every event and heartbeat goes out with the boundary that ends it, see
    // `create_incremental_delivery_stream`
    const BOUNDARY: &str = concat!("--", APOLLO_MULTIPART_HTTP_BOUNDARY);
    const PART_END: &str = concat!("\r\n", BOUNDARY);
    let ping = "{}";
    async_stream::stream! {
        yield Ok(Bytes::from(BOUNDARY));
        loop {
            tokio::select! {
                item = input.next() => {
                    match item {
                        Some(resp) => {
                            match std::str::from_utf8(&resp) {
                                Ok(_) => {
                                    yield Ok(Bytes::from(PART_HEAD));
                                    // Wrap the GraphQL response in a payload field
                                    // As per the spec.
                                    let mut payload = ntex::util::BytesMut::with_capacity(resp.len() + 15);
                                    payload.put_slice(br#"{"payload":"#);
                                    payload.put_slice(&resp);
                                    payload.put_slice(br"}");
                                    yield Ok(payload.freeze());
                                    yield Ok(Bytes::from(PART_END));
                                }
                                Err(e) => {
                                    // TODO: use transport level errors as per spec
                                    yield Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                                    break;
                                }
                            }
                        }
                        None => {
                            yield Ok(Bytes::from(CLOSE));
                            break;
                        },
                    }
                }
                _ = tokio::time::sleep(heartbeat_interval) => {
                    yield Ok(Bytes::from(PART_HEAD));
                    yield Ok(Bytes::from(ping));
                    yield Ok(Bytes::from(PART_END));
                }
            }
        }
    }.boxed()
}

#[cfg(test)]
mod tests {
    use super::{create_apollo_multipart_http_stream, create_incremental_delivery_stream};
    use futures_util::{FutureExt, StreamExt};

    /// What the stream has to give right now, without waiting for more parts.
    fn ready(
        stream: &mut (impl futures_util::Stream<Item = Result<ntex::util::Bytes, std::io::Error>>
                  + Unpin),
    ) -> String {
        let mut out = Vec::new();
        while let Some(Some(chunk)) = stream.next().now_or_never() {
            out.extend_from_slice(&chunk.unwrap());
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn a_part_goes_out_with_the_boundary_that_ends_it() {
        let (parts, input) = futures::channel::mpsc::unbounded::<Vec<u8>>();
        let mut stream = create_incremental_delivery_stream(input);

        // the next part isn't there yet, the client still has all of this one
        parts
            .unbounded_send(br#"{"hasNext":true}"#.to_vec())
            .unwrap();
        assert_eq!(
            ready(&mut stream),
            "---\r\nContent-Type: application/json\r\n\r\n{\"hasNext\":true}\r\n---"
        );

        parts
            .unbounded_send(br#"{"hasNext":false}"#.to_vec())
            .unwrap();
        assert_eq!(
            ready(&mut stream),
            "\r\nContent-Type: application/json\r\n\r\n{\"hasNext\":false}\r\n---"
        );

        drop(parts);
        assert_eq!(ready(&mut stream), "--");
    }

    #[tokio::test]
    async fn a_subscription_event_goes_out_with_the_boundary_that_ends_it() {
        let (events, input) = futures::channel::mpsc::unbounded::<Vec<u8>>();
        let heartbeat = std::time::Duration::from_secs(3600);
        let mut stream = create_apollo_multipart_http_stream(input, heartbeat);

        // no next event and no heartbeat yet, the client still has all of this one
        events
            .unbounded_send(br#"{"data":{"a":1}}"#.to_vec())
            .unwrap();
        assert_eq!(
            ready(&mut stream),
            "--graphql\r\nContent-Type: application/json\r\n\r\n{\"payload\":{\"data\":{\"a\":1}}}\r\n--graphql"
        );

        drop(events);
        assert_eq!(ready(&mut stream), "--");
    }
}
