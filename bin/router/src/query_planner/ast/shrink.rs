//! Deep memory compaction for long-lived planner data.
//!
//! Planner data is built with growable collections and then kept in caches.
//! These collections may keep unused capacity after construction or filtering.
//! Before caching, we recursively shrink owned vectors to reduce retained memory.
//!

/// Recursively releases unused capacity in a value and its children.
pub trait ShrinkMemory {
    fn shrink_memory(&mut self);
}

impl<T: ShrinkMemory> ShrinkMemory for Option<T> {
    #[inline]
    fn shrink_memory(&mut self) {
        if let Some(inner) = self {
            inner.shrink_memory();
        }
    }
}

/// Shrink each item first, then shrink the vector
impl<T: ShrinkMemory> ShrinkMemory for Vec<T> {
    #[inline]
    fn shrink_memory(&mut self) {
        for item in self.iter_mut() {
            item.shrink_memory();
        }
        self.shrink_to_fit();
    }
}
