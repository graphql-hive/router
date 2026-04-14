// TODO: test thoroughly

use futures_util::{Stream, StreamExt};
use ntex::util::Bytes;
use std::time::Duration;
use tokio_util::bytes::BufMut;

pub const SSE_HEADER: &str = "text/event-stream";

pub fn create_stream(
    input: impl Stream<Item = Vec<u8>> + Send + Unpin + 'static,
    heartbeat_interval: Duration,
) -> impl Stream<Item = Result<ntex::util::Bytes, std::io::Error>> + Unpin {
    let mut input = input;
    async_stream::stream! {
        loop {
            tokio::select! {
                item = input.next() => {
                    match item {
                        Some(resp) => {
                             match std::str::from_utf8(&resp) {
                                Ok(json_str) => {
                                    let mut sse_event = ntex::util::BytesMut::with_capacity(json_str.len() + 25);
                                    sse_event.put_slice(b"event: next\ndata: ");
                                    sse_event.put_slice(json_str.as_bytes());
                                    sse_event.put_slice(b"\n\n");
                                    yield Ok(sse_event.freeze());
                                }
                                Err(e) => {
                                    yield Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                                    break;
                                }
                            }
                        }
                        None => {
                            yield Ok(Bytes::from("event: complete\n\n"));
                            break;
                        },
                    }
                }
                _ = tokio::time::sleep(heartbeat_interval) => {
                    yield Ok(Bytes::from(":\n\n"));
                }
            }
        }
    }
    .boxed()
}
