use futures::stream::{BoxStream, Stream};
use futures_util::StreamExt;
use ntex::rt;
use tokio::sync::mpsc;
use tracing::debug;

use hive_router_internal::telemetry::metrics::subscription_metrics::SubscriptionTransport;
use hive_router_internal::telemetry::TelemetryContext;

use crate::executors::error::SubgraphExecutorError;
use crate::response::subgraph_response::SubgraphResponse;

type SubscriptionItem = Result<SubgraphResponse<'static>, SubgraphExecutorError>;

/// Outcome of `try_send_or_drop`, so callers can decide what to do about a closed receiver
/// (the full/dropped case needs no follow-up, it's already been recorded).
pub enum SendOutcome {
    Sent,
    Dropped,
    Closed,
}

/// Central place to handle a full or closed bounded channel when forwarding a single
/// subscription message. On `Full`, the message is dropped (consumer too slow) and recorded
/// via `record_message_dropped`, but the channel/subscription stays alive, mirroring
/// broadcast::Lagged semantics. On `Closed`, the caller is told so it can tear down its side.
pub fn try_send_or_drop<T>(
    tx: &mpsc::Sender<T>,
    item: T,
    telemetry_context: &TelemetryContext,
    transport: SubscriptionTransport,
    subgraph_name: &str,
    endpoint: &str,
) -> SendOutcome {
    match tx.try_send(item) {
        Ok(()) => SendOutcome::Sent,
        Err(mpsc::error::TrySendError::Full(_)) => {
            // drop the message but keep the subscription alive, same as broadcast::Lagged
            // NOTE: not warn to avoid log spam with an active slow consumer. users should rely
            // on the dropped_messages metric to detect slow consumers and tune accordingly
            debug!(
                subgraph_name = %subgraph_name, endpoint = %endpoint,
                "Consumer for subgraph is too slow, dropping message",
            );
            telemetry_context
                .metrics
                .subscriptions
                .record_message_dropped(transport);
            SendOutcome::Dropped
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            // expected teardown path: fires once all downstream clients have
            // unsubscribed/disconnected and the receiver was dropped.
            // not an error, just means there's nothing left to forward to.
            // TODO: since this is expected, is the debug log even necessary?
            debug!(
                subgraph_name = %subgraph_name, endpoint = %endpoint,
                "Subscription buffer for subgraph has no more receivers, all consumers disconnected or unsubscribed; stopping upstream drain",
            );
            SendOutcome::Closed
        }
    }
}

/// Forward every item from `source` into `tx`, dropping messages when the channel is full so
/// the emitting subgraph is never throttled by a slow consumer (entity resolution, slow client,
/// broadcaster lag). Returns when the source ends or the consumer drops the receiver.
///
/// Use this directly when the source is non-Send (e.g. a websocket client holding Rc/RefCell)
/// and must be driven on the caller's local runtime task. For Send sources prefer `buffered`,
/// which spawns the drainer for you.
pub async fn drain_into<S>(
    mut source: S,
    tx: mpsc::Sender<SubscriptionItem>,
    telemetry_context: &TelemetryContext,
    transport: SubscriptionTransport,
    subgraph_name: &str,
    endpoint: &str,
) where
    S: Stream<Item = SubscriptionItem> + Unpin,
{
    while let Some(item) = source.next().await {
        if matches!(
            try_send_or_drop(
                &tx,
                item,
                telemetry_context,
                transport,
                subgraph_name,
                endpoint
            ),
            SendOutcome::Closed
        ) {
            break;
        }
    }
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
    telemetry_context: std::sync::Arc<TelemetryContext>,
    transport: SubscriptionTransport,
    subgraph_name: String,
    endpoint: String,
) -> BoxStream<'static, SubscriptionItem>
where
    S: Stream<Item = SubscriptionItem> + Unpin + 'static,
{
    let (tx, mut rx) = mpsc::channel::<SubscriptionItem>(buffer_size);

    // ntex::rt::spawn keeps the drainer on the local ntex runtime, matching the rest of the
    // subscription pipeline.
    drop(rt::spawn(async move {
        drain_into(
            source,
            tx,
            &telemetry_context,
            transport,
            &subgraph_name,
            &endpoint,
        )
        .await;
    }));

    Box::pin(async_stream::stream! {
        while let Some(item) = rx.recv().await {
            yield item;
        }
    })
}
