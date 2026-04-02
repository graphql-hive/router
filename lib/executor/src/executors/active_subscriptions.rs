use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bytes::Bytes;
use dashmap::DashMap;
use tracing::trace;
use uuid::Uuid;

use crate::response::graphql_error::GraphQLError;

pub type SubscriptionId = String;
pub type Fingerprint = u64;

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
    listener_count: Arc<AtomicUsize>,
    fingerprint: Option<Fingerprint>,
    callback_state: Option<CallbackState>,
}

pub struct ActiveSubscriptionsRegistry {
    subscriptions: DashMap<SubscriptionId, ActiveSubscriptionEntry>,
    fingerprints: DashMap<Fingerprint, SubscriptionId>,
    // capacity of the broadcast channel per subscription, see router config `subscriptions.broadcast_capacity`
    broadcast_capacity: usize,
}

impl ActiveSubscriptionsRegistry {
    pub fn new(broadcast_capacity: usize) -> Self {
        Self {
            subscriptions: DashMap::new(),
            fingerprints: DashMap::new(),
            broadcast_capacity,
        }
    }

    /// try to join an existing subscription by fingerprint.
    /// returns the subscription id, a broadcast receiver, and a listener guard if a match exists
    pub fn try_join_by_fingerprint(
        self: &Arc<Self>,
        fingerprint: Fingerprint,
    ) -> Option<(
        SubscriptionId,
        tokio::sync::broadcast::Receiver<BroadcastItem>,
        ListenerGuard,
    )> {
        let sub_id = self.fingerprints.get(&fingerprint)?.value().clone();

        let entry = self.subscriptions.get(&sub_id)?;
        let receiver = entry.sender.subscribe();
        entry.listener_count.fetch_add(1, Ordering::AcqRel);

        let guard = ListenerGuard {
            id: sub_id.clone(),
            registry: Arc::clone(self),
            listener_count: entry.listener_count.clone(),
            fingerprint: entry.fingerprint,
        };

        trace!(subscription_id = %sub_id, fingerprint = fingerprint, "joined existing subscription via dedup");

        Some((sub_id, receiver, guard))
    }

    /// register a brand new subscription. returns:
    /// - a SubscriptionHandle for the upstream producer (dropping it removes the entry and closes the channel)
    /// - a broadcast receiver for the first consumer
    /// - a ListenerGuard that tracks this consumer's lifetime
    pub fn register(
        self: &Arc<Self>,
        fingerprint: Option<Fingerprint>,
        callback_state: Option<CallbackState>,
    ) -> (
        SubscriptionHandle,
        tokio::sync::broadcast::Receiver<BroadcastItem>,
        ListenerGuard,
    ) {
        let id = Uuid::new_v4().to_string();
        let (sender, receiver) = tokio::sync::broadcast::channel(self.broadcast_capacity);
        let listener_count = Arc::new(AtomicUsize::new(1));

        let entry = ActiveSubscriptionEntry {
            sender,
            listener_count: listener_count.clone(),
            fingerprint,
            callback_state,
        };

        self.subscriptions.insert(id.clone(), entry);

        if let Some(fp) = fingerprint {
            self.fingerprints.insert(fp, id.clone());
        }

        let handle = SubscriptionHandle {
            id: id.clone(),
            registry: Arc::clone(self),
        };

        let guard = ListenerGuard {
            id: id.clone(),
            registry: Arc::clone(self),
            listener_count,
            fingerprint,
        };

        trace!(subscription_id = %id, "registered new subscription");

        (handle, receiver, guard)
    }

    /// check if a subscription exists
    pub fn contains(&self, id: &str) -> bool {
        self.subscriptions.contains_key(id)
    }

    /// get the verifier for a callback subscription
    pub fn get_callback_verifier(&self, id: &str) -> Option<String> {
        self.subscriptions
            .get(id)
            .and_then(|entry| entry.callback_state.as_ref().map(|cs| cs.verifier.clone()))
    }

    /// record a heartbeat for a callback subscription
    pub fn record_heartbeat(&self, id: &str) -> bool {
        if let Some(entry) = self.subscriptions.get(id) {
            if let Some(ref cs) = entry.callback_state {
                cs.record_heartbeat();
                return true;
            }
        }
        false
    }

    /// send an event to a specific subscription's broadcast channel
    pub fn send_event(&self, id: &str, item: BroadcastItem) -> bool {
        if let Some(entry) = self.subscriptions.get(id) {
            entry.sender.send(item).is_ok()
        } else {
            false
        }
    }

    /// remove a subscription entry and clean up its fingerprint mapping
    pub fn remove(&self, id: &str) {
        if let Some((_, entry)) = self.subscriptions.remove(id) {
            if let Some(fp) = entry.fingerprint {
                self.fingerprints.remove(&fp);
            }
        }
    }

    /// close all active subscriptions with an error message
    pub fn close_all_with_error(&self, errors: Vec<GraphQLError>) {
        let item = BroadcastItem::Error(errors);
        for entry in self.subscriptions.iter() {
            let _ = entry.sender.send(item.clone());
        }
        self.fingerprints.clear();
        self.subscriptions.clear();
    }

    /// iterate over all subscription ids and their callback state for heartbeat enforcement
    pub fn iter_callback_subscriptions(
        &self,
    ) -> impl Iterator<Item = (SubscriptionId, Arc<Mutex<Instant>>)> + '_ {
        self.subscriptions.iter().filter_map(|entry| {
            entry
                .callback_state
                .as_ref()
                .map(|cs| (entry.key().clone(), cs.last_heartbeat.clone()))
        })
    }
}

/// held by the upstream producer (the task that reads from the subgraph).
/// dropping this removes the subscription entry from the registry, which drops
/// the broadcast sender and closes the channel. all receivers will see Closed
/// and their streams will end naturally
pub struct SubscriptionHandle {
    id: SubscriptionId,
    registry: Arc<ActiveSubscriptionsRegistry>,
}

impl SubscriptionHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// send an event to all listeners of this subscription
    pub fn send(&self, item: BroadcastItem) -> bool {
        self.registry.send_event(&self.id, item)
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        // removing the entry drops the broadcast sender inside it, closing the channel.
        // all receivers will see Closed and their ListenerGuards will drop.
        // we remove here (rather than in ListenerGuard) because the upstream is the
        // authoritative source - when it's gone, the subscription is done
        self.registry.remove(&self.id);
        trace!(subscription_id = %self.id, "subscription handle dropped, upstream closed");
    }
}

/// held by each consumer. on drop, decrements the listener count.
/// when the last listener drops and the subscription entry still exists
/// (upstream hasn't dropped yet), removes it - causing the upstream
/// task to see send() fail and exit
pub struct ListenerGuard {
    id: SubscriptionId,
    registry: Arc<ActiveSubscriptionsRegistry>,
    listener_count: Arc<AtomicUsize>,
    fingerprint: Option<Fingerprint>,
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        let prev = self.listener_count.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 {
            // last listener gone, clean up. this also drops the sender,
            // causing the upstream producer's send() to return false
            self.registry.subscriptions.remove(&self.id);
            if let Some(fp) = self.fingerprint {
                self.registry.fingerprints.remove(&fp);
            }
            trace!(subscription_id = %self.id, "last listener dropped, subscription removed");
        } else {
            trace!(subscription_id = %self.id, remaining = prev - 1, "listener dropped");
        }
    }
}
