use std::sync::Arc;

use bytes::Bytes;
use dashmap::DashMap;
use hive_router_plan_executor::response::graphql_error::GraphQLError;
use tokio::sync::broadcast;
use tracing::trace;
use ulid::Ulid;

use crate::shared_state::SharedRouterResponseGuard;

pub type SubscriptionId = String;

#[derive(Clone, Debug)]
pub enum SubscriptionEvent {
    /// A normal subscription event from the upstream, already serialized.
    /// Uses Bytes for zero-copy cloning across broadcast receivers.
    Raw(Bytes),
    /// An error pushed externally (e.g. supergraph reload, shutdown).
    /// Consumers should yield this as the final event and then stop.
    Error(Vec<GraphQLError>),
}

#[derive(Clone)]
pub struct ActiveSubscriptions {
    map: Arc<DashMap<SubscriptionId, broadcast::Sender<SubscriptionEvent>>>,
    broadcast_capacity: usize,
}

impl ActiveSubscriptions {
    pub fn new(broadcast_capacity: usize) -> Self {
        Self {
            map: Arc::new(DashMap::new()),
            broadcast_capacity,
        }
    }

    /// Register a new active subscription. Returns a producer handle for the upstream pump
    /// and a pre-subscribed receiver for the leader consumer. The pump task owns the handle
    /// for the full lifetime of the upstream stream - when the handle drops (pump done or all
    /// receivers gone) the broadcast channel closes and all consumer receivers terminate.
    pub fn register(
        &self,
        guard: Option<SharedRouterResponseGuard>,
    ) -> (ProducerHandle, broadcast::Receiver<SubscriptionEvent>) {
        let (sender, receiver) = broadcast::channel(self.broadcast_capacity);
        let id = Ulid::new().to_string();
        self.map.insert(id.clone(), sender.clone());

        let handle = ProducerHandle {
            id: id.clone(),
            map: self.map.clone(),
            sender,
            _guard: guard,
        };

        trace!(subscription_id = %id, "registered new subscription");

        (handle, receiver)
    }

    /// Close all active subscriptions with an error and clear the registry.
    pub fn close_all_with_error(&self, errors: Vec<GraphQLError>) {
        let item = SubscriptionEvent::Error(errors);
        for entry in self.map.iter() {
            let _ = entry.send(item.clone());
        }
        self.map.clear();
    }
}

/// Held by the upstream pump task for the full lifetime of the stream. Dropping it removes
/// the subscription from the registry, closes the broadcast channel, and drops the inflight
/// cleanup guard - which removes the dedupe entry so new requests start a fresh upstream.
pub struct ProducerHandle {
    id: SubscriptionId,
    map: Arc<DashMap<SubscriptionId, broadcast::Sender<SubscriptionEvent>>>,
    sender: broadcast::Sender<SubscriptionEvent>,
    _guard: Option<SharedRouterResponseGuard>,
}

impl ProducerHandle {
    pub fn sender(&self) -> &broadcast::Sender<SubscriptionEvent> {
        &self.sender
    }

    /// Returns false when all consumers have gone and the event cannot be delivered.
    pub fn send(&self, item: SubscriptionEvent) -> bool {
        self.sender.send(item).is_ok()
    }
}

impl Drop for ProducerHandle {
    fn drop(&mut self) {
        self.map.remove(&self.id);
        trace!(subscription_id = %self.id, "producer dropped, upstream closed");
    }
}
