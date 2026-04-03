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
    // two maps becuase http callbacks need uuids as ids which is the
    // true map of all subscriptions, and then the fingerprint map is a
    // secondary map only for deduplication
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

    /// Atomically joins or creates a subscription for the given fingerprint.
    ///
    /// The broadcast entry is created immediately so concurrent requests
    /// subscribe to the same channel, even if the subscription has not
    /// resolved yet.
    ///
    /// Returns a `SubscriptionHandle` only for the leader (first caller).
    /// the leader uses it to feed upstream events into the broadcast channel.
    ///
    /// All callers (leader included) get a receiver and a `FingerprintGuard`.
    pub fn dedupe_by_fingerprint(
        self: &Arc<Self>,
        fingerprint: Fingerprint,
    ) -> (
        Option<SubscriptionHandle>,
        tokio::sync::broadcast::Receiver<BroadcastItem>,
        FingerprintGuard,
    ) {
        use dashmap::mapref::entry::Entry;
        match self.fingerprints.entry(fingerprint) {
            Entry::Occupied(entry) => {
                let sub_id = entry.get().clone();
                // unwrap: fingerprints and subscriptions are always in sync
                let sub = self.subscriptions.get(&sub_id).unwrap();
                let receiver = sub.sender.subscribe();
                sub.listener_count.fetch_add(1, Ordering::AcqRel);

                let guard = FingerprintGuard {
                    id: sub_id.clone(),
                    registry: Arc::clone(self),
                    listener_count: sub.listener_count.clone(),
                    fingerprint: Some(fingerprint),
                };

                trace!(subscription_id = %sub_id, fingerprint, "joined existing subscription via dedup");

                (None, receiver, guard)
            }
            Entry::Vacant(fp_slot) => {
                let id = Uuid::new_v4().to_string(); // TODO: doesnt have to be a UUID
                let (sender, receiver) = tokio::sync::broadcast::channel(self.broadcast_capacity);
                let listener_count = Arc::new(AtomicUsize::new(1));

                self.subscriptions.insert(
                    id.clone(),
                    ActiveSubscriptionEntry {
                        sender,
                        listener_count: listener_count.clone(),
                        fingerprint: Some(fingerprint),
                        callback_state: None,
                    },
                );
                fp_slot.insert(id.clone());

                let handle = SubscriptionHandle {
                    id: id.clone(),
                    registry: Arc::clone(self),
                };
                let guard = FingerprintGuard {
                    id: id.clone(),
                    registry: Arc::clone(self),
                    listener_count,
                    fingerprint: Some(fingerprint),
                };

                trace!(subscription_id = %id, fingerprint, "registered new fingerprinted subscription");

                (Some(handle), receiver, guard)
            }
        }
    }

    /// register a subscription without dedup (e.g. http callbacks)
    pub fn register(
        self: &Arc<Self>,
        fingerprint: Option<Fingerprint>,
        callback_state: Option<CallbackState>,
    ) -> (
        SubscriptionHandle,
        tokio::sync::broadcast::Receiver<BroadcastItem>,
        FingerprintGuard,
    ) {
        let id = Uuid::new_v4().to_string(); // TODO: doesnt have to be a UUID
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

        let guard = FingerprintGuard {
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
        // all receivers will see Closed and their FingerprintGuards will drop.
        // we remove here (rather than in FingerprintGuard) because the upstream is the
        // authoritative source - when it's gone, the subscription is done
        self.registry.remove(&self.id);
        trace!(subscription_id = %self.id, "subscription handle dropped, upstream closed");
    }
}

/// held by each consumer of a subscription. on drop, decrements the listener
/// count. when the last guard drops and the subscription entry still exists
/// (upstream hasn't dropped yet), removes it - causing the upstream
/// producer's send() to return false and exit
pub struct FingerprintGuard {
    id: SubscriptionId,
    registry: Arc<ActiveSubscriptionsRegistry>,
    listener_count: Arc<AtomicUsize>,
    fingerprint: Option<Fingerprint>,
}

impl Drop for FingerprintGuard {
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
