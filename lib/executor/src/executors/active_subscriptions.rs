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
    /// a normal subscription event from the upstream, already serialized.
    /// uses Bytes for zero-copy cloning across broadcast receivers
    Event(Bytes),
    /// a terminal error pushed externally (e.g. supergraph reload, shutdown).
    /// consumers should yield this as the final event and then stop
    Error(Vec<GraphQLError>),
}

/// state specific to http callback subscriptions
pub struct CallbackState {
    pub verifier: String,
    pub last_heartbeat: Arc<Mutex<Instant>>,
}

impl CallbackState {
    pub fn record_heartbeat(&self) {
        *self.last_heartbeat.lock().unwrap() = Instant::now();
    }
}

struct ActiveSubscriptionEntry {
    sender: tokio::sync::broadcast::Sender<BroadcastItem>,
    callback_state: Option<CallbackState>,
}

struct ActiveSubscriptionsInner {
    subscriptions: DashMap<SubscriptionId, ActiveSubscriptionEntry>,
    // capacity of the broadcast channel per subscription, see router config `subscriptions.broadcast_capacity`
    broadcast_capacity: usize,
}

/// cheap to clone - all clones share the same inner state
#[derive(Clone)]
pub struct ActiveSubscriptionsMap {
    inner: Arc<ActiveSubscriptionsInner>,
}

impl ActiveSubscriptionsMap {
    pub fn new(broadcast_capacity: usize) -> Self {
        Self {
            inner: Arc::new(ActiveSubscriptionsInner {
                subscriptions: DashMap::new(),
                broadcast_capacity,
            }),
        }
    }

    /// Register a new subscription (e.g. http callbacks).
    /// Always creates a new entry. Deduplication for fingerprinted subscriptions is handled
    /// by the inflight map in the request pipeline, not here.
    pub fn register(
        &self,
        callback_state: Option<CallbackState>,
    ) -> (
        SubscriptionHandle,
        tokio::sync::broadcast::Receiver<BroadcastItem>,
        ListenerGuard,
    ) {
        let id = Ulid::new().to_string();
        let (sender, receiver) = tokio::sync::broadcast::channel(self.inner.broadcast_capacity);
        let listener_count = Arc::new(AtomicUsize::new(1));

        self.inner.subscriptions.insert(
            id.clone(),
            ActiveSubscriptionEntry {
                sender,
                callback_state,
            },
        );

        let handle = SubscriptionHandle {
            id: id.clone(),
            map: self.clone(),
        };
        let guard = ListenerGuard {
            id: id.clone(),
            map: self.clone(),
            listener_count,
        };

        trace!(subscription_id = %id, "registered new subscription");

        (handle, receiver, guard)
    }

    /// check if a subscription exists
    pub fn contains(&self, id: &str) -> bool {
        self.inner.subscriptions.contains_key(id)
    }

    /// get the verifier for a callback subscription
    pub fn get_callback_verifier(&self, id: &str) -> Option<String> {
        self.inner
            .subscriptions
            .get(id)
            .and_then(|entry| entry.callback_state.as_ref().map(|cs| cs.verifier.clone()))
    }

    /// record a heartbeat for a callback subscription
    pub fn record_heartbeat(&self, id: &str) -> bool {
        if let Some(entry) = self.inner.subscriptions.get(id) {
            if let Some(ref cs) = entry.callback_state {
                cs.record_heartbeat();
                return true;
            }
        }
        false
    }

    /// send an event to a specific subscription's broadcast channel
    pub fn send_event(&self, id: &str, item: BroadcastItem) -> bool {
        if let Some(entry) = self.inner.subscriptions.get(id) {
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
        self.inner.subscriptions.remove(id);
    }

    /// close all active subscriptions with an error message
    pub fn close_all_with_error(&self, errors: Vec<GraphQLError>) {
        let item = BroadcastItem::Error(errors);
        for entry in self.inner.subscriptions.iter() {
            let _ = entry.sender.send(item.clone());
        }
        self.inner.subscriptions.clear();
    }

    /// iterate over all subscription ids and their callback state for heartbeat enforcement
    pub fn iter_callback_subscriptions(
        &self,
    ) -> impl Iterator<Item = (SubscriptionId, Arc<Mutex<Instant>>)> + '_ {
        self.inner.subscriptions.iter().filter_map(|entry| {
            entry
                .callback_state
                .as_ref()
                .map(|cs| (entry.key().clone(), cs.last_heartbeat.clone()))
        })
    }
}

/// Held by the upstream producer (the task that reads from the subgraph).
/// Dropping this removes the subscription entry from the registry, which drops
/// the broadcast sender and closes the channel. All receivers will see `Closed`
/// and their streams will end naturally.
pub struct SubscriptionHandle {
    id: SubscriptionId,
    map: ActiveSubscriptionsMap,
}

impl SubscriptionHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn send(&self, item: BroadcastItem) -> bool {
        self.map.send_event(&self.id, item)
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        // removing the entry drops the broadcast sender inside it, closing the channel.
        // all receivers will see Closed and their streams will end naturally
        self.map.remove(&self.id);
        trace!(subscription_id = %self.id, "subscription handle dropped, upstream closed");
    }
}

/// Held by each consumer of a subscription. On drop, decrements the listener count.
/// When the last guard drops and the subscription entry still exists (upstream hasn't dropped
/// yet), removes it - causing the upstream producer's `send()` to return `false` and exit.
pub struct ListenerGuard {
    id: SubscriptionId,
    map: ActiveSubscriptionsMap,
    listener_count: Arc<AtomicUsize>,
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        let prev = self.listener_count.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 {
            // last listener gone, clean up. this also drops the sender,
            // causing the upstream producer's send() to return false
            self.map.inner.subscriptions.remove(&self.id);
            trace!(subscription_id = %self.id, "last listener dropped, subscription removed");
        } else {
            trace!(subscription_id = %self.id, remaining = prev - 1, "listener dropped");
        }
    }
}
