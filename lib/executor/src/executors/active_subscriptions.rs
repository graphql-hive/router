use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bytes::Bytes;
use dashmap::DashMap;
use tracing::trace;
use ulid::Ulid;

use crate::response::graphql_error::GraphQLError;

pub type SubscriptionId = String;

#[derive(Clone, Debug)]
pub enum BroadcastItem {
    /// A normal subscription event from the upstream, already serialized.
    /// Uses Bytes for zero-copy cloning across broadcast receivers.
    Event(Bytes),
    /// An error pushed externally (e.g. supergraph reload, shutdown).
    /// Consumers should yield this as the final event and then stop.
    Error(Vec<GraphQLError>),
}

pub struct CallbackState {
    pub verifier: String,
    pub last_heartbeat: Arc<Mutex<Instant>>,
}

impl CallbackState {
    pub fn record_heartbeat(&self) {
        *self.last_heartbeat.lock().unwrap() = Instant::now();
    }
}

struct Subscription {
    sender: tokio::sync::broadcast::Sender<BroadcastItem>,
    /// The optional callback state that is only present for http callback subscriptions.
    callback_state: Option<CallbackState>,
}

#[derive(Clone)]
pub struct ActiveSubscriptions {
    // map of subscription ids to their sender
    map: Arc<DashMap<SubscriptionId, Subscription>>,
    // capacity of the broadcast channel per subscription,
    // see router config `subscriptions.broadcast_capacity`
    broadcast_capacity: usize,
}

impl ActiveSubscriptions {
    pub fn new(broadcast_capacity: usize) -> Self {
        Self {
            map: Arc::new(DashMap::new()),
            broadcast_capacity,
        }
    }

    /// Register a new subscription to be used for broadcasting events to consuming clients.
    /// Returns a handle for the producer to send events, a sender that can be cloned to subscribe
    /// new receivers, a receiver for consumers to subscribe to, and a guard that each consumer
    /// must hold onto while consuming.
    pub fn register(
        &self,
        guard: Option<Box<dyn std::any::Any + Send + 'static>>,
        callback_state: Option<CallbackState>,
    ) -> (
        ProducerHandle,
        tokio::sync::broadcast::Sender<BroadcastItem>,
        tokio::sync::broadcast::Receiver<BroadcastItem>,
        ConsumerGuard,
    ) {
        let listener_count = Arc::new(AtomicUsize::new(1));
        let (sender, receiver) = tokio::sync::broadcast::channel(self.broadcast_capacity);
        let sender_clone = sender.clone();

        let id = Ulid::new().to_string();

        self.map.insert(
            id.clone(),
            Subscription {
                sender,
                callback_state,
            },
        );

        let handle = ProducerHandle {
            id: id.clone(),
            map: self.clone(),
            _guard: guard,
        };
        let guard = ConsumerGuard {
            id: id.clone(),
            map: self.clone(),
            listener_count,
        };

        trace!(subscription_id = %id, "registered new subscription");

        (handle, sender_clone, receiver, guard)
    }

    /// check if a subscription exists
    pub fn contains(&self, id: &str) -> bool {
        self.map.contains_key(id)
    }

    /// get the verifier for a callback subscription
    pub fn get_callback_verifier(&self, id: &str) -> Option<String> {
        self.map
            .get(id)
            .and_then(|entry| entry.callback_state.as_ref().map(|cs| cs.verifier.clone()))
    }

    /// record a heartbeat for a callback subscription
    pub fn record_heartbeat(&self, id: &str) -> bool {
        if let Some(entry) = self.map.get(id) {
            if let Some(ref cs) = entry.callback_state {
                cs.record_heartbeat();
                return true;
            }
        }
        false
    }

    /// send an event to a specific subscription's broadcast channel
    pub fn send_event(&self, id: &str, item: BroadcastItem) -> bool {
        if let Some(entry) = self.map.get(id) {
            // if the channel is closed or full it means the consuming client is gone or too slow and
            // unable to keep up. in both cases, we dont emit an error messages because it anyways cant
            // go through
            entry.sender.send(item).is_ok()
        } else {
            false
        }
    }

    /// remove a subscription entry
    pub fn remove(&self, id: &str) {
        self.map.remove(id);
    }

    /// close all active subscriptions with an error message
    pub fn close_all_with_error(&self, errors: Vec<GraphQLError>) {
        let item = BroadcastItem::Error(errors);
        for entry in self.map.iter() {
            let _ = entry.sender.send(item.clone());
        }
        self.map.clear();
    }

    /// iterate over all subscription ids and their callback state for heartbeat enforcement
    pub fn iter_callback_subscriptions(
        &self,
    ) -> impl Iterator<Item = (SubscriptionId, Arc<Mutex<Instant>>)> + '_ {
        self.map.iter().filter_map(|entry| {
            entry
                .callback_state
                .as_ref()
                .map(|cs| (entry.key().clone(), cs.last_heartbeat.clone()))
        })
    }
}

/// Held by the upstream producer (the task that reads from the subgraph).
/// It is the actual subscription handle that can be used to send events to consumers.
/// Dropping this removes the subscription entry from the registry, which drops
/// the broadcast sender and closes the channel. All receivers will see `Closed`
/// and their streams will end naturally.
pub struct ProducerHandle {
    id: SubscriptionId,
    map: ActiveSubscriptions,
    _guard: Option<Box<dyn std::any::Any + Send>>,
}

impl ProducerHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn send(&self, item: BroadcastItem) -> bool {
        self.map.send_event(&self.id, item)
    }
}

impl Drop for ProducerHandle {
    fn drop(&mut self) {
        // removing the entry drops the broadcast sender inside it, closing the channel.
        // all receivers will see Closed and their streams will end naturally
        self.map.remove(&self.id);
        trace!(subscription_id = %self.id, "subscription handle dropped, upstream closed");
    }
}

/// Held by each consumer of a subscription (producer). On drop, decrements the listener count.
/// When the last guard drops and the subscription entry still exists (upstream hasn't dropped
/// yet), removes it - causing the upstream producer's `send()` to return `false` and exit.
pub struct ConsumerGuard {
    id: SubscriptionId,
    map: ActiveSubscriptions,
    listener_count: Arc<AtomicUsize>,
}

impl Drop for ConsumerGuard {
    fn drop(&mut self) {
        let prev = self.listener_count.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 {
            // last listener gone, clean up. this also drops the sender,
            // causing the upstream producer's send() to return false
            self.map.map.remove(&self.id);
            trace!(subscription_id = %self.id, "last listener dropped, subscription removed");
        } else {
            trace!(subscription_id = %self.id, remaining = prev - 1, "listener dropped");
        }
    }
}
