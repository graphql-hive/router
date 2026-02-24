// TODO: test thoroughly

use const_str::concat;
use futures_timer::Delay;
use futures_util::{FutureExt, Stream, StreamExt};
use ntex::util::Bytes;
use std::time::Duration;
use tokio_util::bytes::BufMut;

// we use macros to retain constness
macro_rules! make_content_type {
    ($boundary:expr) => {
        concat!("multipart/mixed;boundary=", $boundary)
    };
}
macro_rules! make_boundaries {
    ($boundary:expr) => {
        (
            // start
            concat!(
                "--",
                $boundary,
                "\r\nContent-Type: application/json\r\n\r\n"
            ),
            // end
            concat!("--", $boundary, "--"),
        )
    };
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
    let mut input = input.fuse();
    let (start_boundary, end_boundary) = make_boundaries!(INCREMENTAL_DELIVERY_BOUNDARY);
    async_stream::stream! {
        loop {
            match input.next().await {
                Some(resp) => {
                    match std::str::from_utf8(&resp) {
                        Ok(_) => {
                            yield Ok(Bytes::from(start_boundary));
                            yield Ok(Bytes::from(resp));
                            yield Ok(Bytes::from("\r\n"));
                        }
                        Err(e) => {
                            yield Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                            break;
                        }
                    }
                }
                None => {
                    yield Ok(Bytes::from(end_boundary));
                    break;
                },
            }
        }
    }
    .boxed()
}

const APOLLO_MULTIPART_HTTP_BOUNDARY: &str = "graphql";

pub const APOLLO_MULTIPART_HTTP_CONTENT_TYPE: &str =
    make_content_type!(APOLLO_MULTIPART_HTTP_BOUNDARY);

/// Create a multipart subscription stream following Apollo's Multipart HTTP spec.
///
/// Will use `graphql` as boundary.
///
/// Read more: https://github.com/graphql/graphql-over-http/blob/d312e43384006fa323b918d49cfd9fbd76ac1257/rfcs/IncrementalDelivery.md
pub fn create_apollo_multipart_http_stream(
    input: impl Stream<Item = Vec<u8>> + Send + Unpin + 'static,
    heartbeat_interval: Duration,
) -> impl Stream<Item = Result<ntex::util::Bytes, std::io::Error>> + Unpin {
    let mut input = input.fuse();
    let mut heartbeat_timer = Delay::new(heartbeat_interval).fuse();
    let (start_boundary, end_boundary) = make_boundaries!(APOLLO_MULTIPART_HTTP_BOUNDARY);
    let ping = "{}\r\n";
    async_stream::stream! {
        loop {
            futures_util::select! {
                item = input.next() => {
                    match item {
                        Some(resp) => {
                            match std::str::from_utf8(&resp) {
                                Ok(_) => {
                                    yield Ok(Bytes::from(start_boundary));
                                    // Wrap the GraphQL response in a payload field
                                    // As per the spec.
                                    let mut payload = ntex::util::BytesMut::with_capacity(resp.len() + 15);
                                    payload.put_slice(br#"{"payload":"#);
                                    payload.put_slice(&resp);
                                    payload.put_slice(br"}");
                                    yield Ok(payload.freeze());
                                    yield Ok(Bytes::from("\r\n"));
                                }
                                Err(e) => {
                                    // TODO: transport level errors as per spec
                                    yield Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                                    break;
                                }
                            }
                        }
                        None => {
                            yield Ok(Bytes::from(end_boundary));
                            break;
                        },
                    }
                }
                _ = heartbeat_timer => {
                    heartbeat_timer = Delay::new(heartbeat_interval).fuse();
                    yield Ok(Bytes::from(start_boundary));
                    yield Ok(Bytes::from(ping));
                }
            }
        }
    }.boxed()
}
