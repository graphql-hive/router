use futures::stream::{BoxStream, Stream};
use futures_util::StreamExt;
use ntex::rt;
use tokio::sync::mpsc;
use tracing::{error, warn};

use crate::executors::error::SubgraphExecutorError;
use crate::response::subgraph_response::SubgraphResponse;

type SubscriptionItem = Result<SubgraphResponse<'static>, SubgraphExecutorError>;

/// Forward every item from `source` into `tx`, dropping messages when the channel is full so
/// the emitting subgraph is never throttled by a slow consumer (entity resolution, slow client,
/// broadcaster lag). Dropping mirrors the broadcaster's drop-on-lag behavior and keeps the
/// subscription alive. Returns when the source ends or the consumer drops the receiver.
///
/// Use this directly when the source is non-Send (e.g. a websocket client holding Rc/RefCell)
/// and must be driven on the caller's local runtime task. For Send sources prefer `buffered`,
/// which spawns the drainer for you.
pub async fn drain_into<S>(
    mut source: S,
    tx: mpsc::Sender<SubscriptionItem>,
    subgraph_name: &str,
    endpoint: &str,
) where
    S: Stream<Item = SubscriptionItem> + Unpin,
{
    while let Some(item) = source.next().await {
        match tx.try_send(item) {
            Ok(()) => (),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // drop the message but keep the subscription alive, same as broadcast::Lagged
                warn!(
                    "Consumer for subgraph {} at {} is too slow, dropping message",
                    subgraph_name, endpoint
                );
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                error!(
                    "Consumer for subgraph {} at {} dropped the receiver",
                    subgraph_name, endpoint
                );
                break;
            }
        }
    }
}

/// Wrap a receiver into a stream the consumer reads from.
pub fn receiver_stream(
    mut rx: mpsc::Receiver<SubscriptionItem>,
) -> BoxStream<'static, SubscriptionItem> {
    Box::pin(async_stream::stream! {
        while let Some(item) = rx.recv().await {
            yield item;
        }
    })
}

/// Decouple a Send subscription source from its slow consumer so the emitting subgraph is never
/// throttled by downstream latency. Spawns a drainer that forwards `source` into a bounded
/// channel with drop-on-full semantics (see `drain_into`) and returns the consumer-side stream.
///
/// `buffer_size` is the channel capacity. Pass `1` for minimal buffering with immediate drop
/// under backpressure.
pub fn buffered<S>(
    source: S,
    buffer_size: usize,
    subgraph_name: String,
    endpoint: String,
) -> BoxStream<'static, SubscriptionItem>
where
    S: Stream<Item = SubscriptionItem> + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel::<SubscriptionItem>(buffer_size);

    // ntex::rt::spawn keeps the drainer on the local ntex runtime, matching the rest of the
    // subscription pipeline.
    drop(rt::spawn(async move {
        drain_into(source, tx, &subgraph_name, &endpoint).await;
    }));

    receiver_stream(rx)
}
