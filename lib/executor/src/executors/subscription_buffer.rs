use std::sync::Arc;

use futures::stream::{BoxStream, Stream};
use futures::StreamExt;
use hive_router_internal::telemetry::TelemetryContext;
use ntex::rt;
use tokio::sync::mpsc;

use crate::executors::error::SubgraphExecutorError;
use crate::response::subgraph_response::SubgraphResponse;

/// An item produced by a subgraph subscription stream.
pub type SubscriptionItem = Result<SubgraphResponse<'static>, SubgraphExecutorError>;

/// Forwards a subgraph subscription event into the bounded buffer shared by all subscription
/// transports.
///
/// The router must read events off the subgraph as they arrive (graphql-ws ping/pong liveness and
/// connection multiplexing forbid back-pressuring a WebSocket socket; HTTP bodies are drained by an
/// eager pump), so back-pressure is applied here instead: on a full buffer - the downstream
/// consumer (per-event plan execution) has fallen behind - the incoming event is dropped
/// (drop-latest), `hive.router.subscription.dropped_events_total` is incremented, and the
/// subscription is kept alive. Returns `false` only when the receiver is gone, signalling the
/// producer to stop.
pub(crate) fn forward_or_drop(
    tx: &mpsc::Sender<SubscriptionItem>,
    item: SubscriptionItem,
    subgraph_name: &str,
    telemetry_context: &TelemetryContext,
) -> bool {
    match tx.try_send(item) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => {
            telemetry_context
                .metrics
                .subscription
                .record_dropped_event(subgraph_name);
            true
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

/// Eagerly drains `source` into a bounded buffer and exposes it as a `Send` stream, applying the
/// shared drop-latest policy via [`forward_or_drop`].
///
/// Used for transports whose source stream is already available (HTTP SSE/multipart). The
/// WebSocket executor drives its own non-`Send` source on the ntex runtime and calls
/// [`forward_or_drop`] directly, but applies the identical policy.
pub(crate) fn buffered_drop_latest<S>(
    source: S,
    capacity: usize,
    subgraph_name: String,
    telemetry_context: Arc<TelemetryContext>,
) -> BoxStream<'static, SubscriptionItem>
where
    S: Stream<Item = SubscriptionItem> + 'static,
{
    let (tx, mut rx) = mpsc::channel::<SubscriptionItem>(capacity.max(1));

    drop(rt::spawn(async move {
        futures::pin_mut!(source);
        while let Some(item) = source.next().await {
            if !forward_or_drop(&tx, item, &subgraph_name, &telemetry_context) {
                break;
            }
        }
    }));

    Box::pin(async_stream::stream! {
        while let Some(item) = rx.recv().await {
            yield item;
        }
    })
}
