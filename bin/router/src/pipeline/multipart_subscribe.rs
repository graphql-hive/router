use futures_timer::Delay;
use futures_util::{FutureExt, Stream, StreamExt};
use ntex::util::Bytes;
use std::time::Duration;
use tokio_util::bytes::BufMut;

/// Create a multipart subscription stream following Apollo's Multipart spec.
/// https://www.apollographql.com/docs/graphos/routing/operations/subscriptions/multipart-protocol
pub fn create_stream(
    input: impl Stream<Item = Vec<u8>> + Send + Unpin + 'static,
    heartbeat_interval: Duration,
) -> impl Stream<Item = Result<ntex::util::Bytes, std::io::Error>> + Unpin {
    let mut input = input.fuse();
    let mut heartbeat_timer = Delay::new(heartbeat_interval).fuse();
    async_stream::stream! {
        loop {
            futures_util::select! {
                item = input.next() => {
                    match item {
                        Some(resp) => {
                            match std::str::from_utf8(&resp) {
                                Ok(_) => {
                                    yield Ok(Bytes::from("--graphql\r\nContent-Type: application/json\r\n\r\n"));
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
                            yield Ok(Bytes::from("--graphql--\r\n"));
                            break;
                        },
                    }
                }
                _ = heartbeat_timer => {
                    heartbeat_timer = Delay::new(heartbeat_interval).fuse();
                    yield Ok(Bytes::from("--graphql\r\nContent-Type: application/json\r\n\r\n"));
                    yield Ok(Bytes::from("{}\r\n"));
                }
            }
        }
    }.boxed()
}
