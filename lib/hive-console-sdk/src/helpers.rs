use ahash::HashSet;
use std::sync::Mutex;

/// A shared, thread-safe version of [`FifoSet`].
pub struct SharedFifoSet {
    inner: Mutex<FifoSet>,
}

impl SharedFifoSet {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(FifoSet::new(capacity)),
        }
    }

    /// See [`FifoSet::insert`].
    pub fn insert(&self, value: u64) -> bool {
        let mut inner = self
            .inner
            .lock()
            .expect("Failed to acquire lock on SharedFifoSet");
        inner.insert(value)
    }
}

/// FIFO set that evicts the oldest entry when full.
///
/// The idea here is to keep the least amount of memory footprint possible,
/// while still providing fast lookup and eviction of the oldest entry when full.
///
/// Why this struct and not let's say Moka?
///
/// Moka's Cache eats 256 bytes per entry,
/// This implementation takes 16 bytes per entry.
/// For 100k entries, this implementation uses ~1.6MB, compared to ~24MB for Moka.
pub struct FifoSet {
    /// Vector stores the values in the order they were inserted.
    eviction_order: Vec<u64>,
    /// Set stores the values for fast lookup.
    seen: HashSet<u64>, // Yeah, we duplicate the memory footprint, but it's still much less than LRU or Moka
    /// The next slot to evict when the set is full.
    next_evict: u32,
    /// The capacity of the set.
    capacity: u32,
}

impl FifoSet {
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity > 0 && capacity < 10_000_000_usize,
            "capacity must be between 1 and 10M"
        );
        Self {
            eviction_order: Vec::new(),
            seen: HashSet::default(),
            next_evict: 0,
            capacity: capacity as u32,
        }
    }

    /// Insert if absent. Returns true if inserted.
    /// When full, evicts the oldest entry to make room.
    #[inline]
    pub fn insert(&mut self, value: u64) -> bool {
        if self.seen.contains(&value) {
            return false;
        }

        if self.seen.len() == self.capacity as usize {
            self.seen
                .remove(&self.eviction_order[self.next_evict as usize]);
            // Replace the oldest element
            self.eviction_order[self.next_evict as usize] = value;
        } else {
            // Not full, so add
            self.eviction_order.push(value);
        }

        self.seen.insert(value);
        self.next_evict = (self.next_evict + 1) % self.capacity;

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fifo_set_overflow() {
        let capacity = 3;
        let mut set = FifoSet::new(capacity);

        assert!(set.insert(1));
        assert!(set.insert(2));
        assert!(set.insert(3));

        // Should not insert
        assert!(!set.insert(3));

        // Should be full
        assert_eq!(set.seen.len(), 3);

        // This should trigger eviction of the oldest (1)
        assert!(set.insert(4));
        assert!(!set.seen.contains(&1));
        assert!(set.seen.contains(&4));

        assert!(set.insert(5));
        assert!(!set.seen.contains(&2));
        assert!(set.seen.contains(&5));
    }
}
