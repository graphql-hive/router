use std::cell::RefCell;

use bumpalo::Bump;

/// The bump arena a response tree is allocated from.
///
/// `Value` owns nothing: strings borrow the response bytes or this arena, and both slice
/// variants borrow this arena. That is what makes a response tree free to drop — there is no
/// `Drop` glue to run — but it also makes the arena and the tree a self-referential pair, so
/// the borrow has to be laundered to `'static`, exactly as the response `Bytes` already are.
///
/// This is sound as long as the arena outlives the tree, which is arranged by storing them
/// together: a subgraph response keeps the arena it parsed into, and both are handed to
/// `ResponsesStorage` when the response is merged into `ctx.data`. The chunks a `Bump` hands
/// out are separately heap-allocated and never move when the `Bump` itself does; boxing it
/// pins the header too, so a `&'static Bump` taken here stays valid even if this struct is
/// moved into a `Vec` that later reallocates.
///
/// # Reuse, and what it costs
///
/// Arenas are pooled per thread rather than allocated per response: `Bump::reset` keeps the
/// largest chunk and frees the rest, so after warm-up a response's whole tree comes from
/// memory that is already mapped and already in cache. Requests are served on one thread, so
/// the pool needs no synchronisation.
///
/// This sharpens the consequence of getting the lifetime discipline wrong. A value that
/// outlived its arena used to read freed memory; now it reads **whatever the next request put
/// there**, which is a cross-request data leak rather than a crash. The discipline is
/// unchanged — an arena is returned exactly where it used to be dropped, at the end of the
/// request that owns it — but debug builds scribble over the retained chunk on the way back
/// into the pool so that a stale read is visibly wrong in tests rather than plausible.
pub struct ResponseArena(Option<Box<Bump>>);

thread_local! {
    /// Idle arenas for this thread. Bounded so a burst does not pin memory forever.
    static ARENA_POOL: RefCell<Vec<Box<Bump>>> = const { RefCell::new(Vec::new()) };
}

/// Arenas kept idle per thread. Each holds one chunk, so this bounds retained memory to
/// roughly `POOL_CAPACITY * MAX_RETAINED_BYTES` per worker.
const POOL_CAPACITY: usize = 16;

/// An arena that grew past this is dropped instead of pooled: one pathological response
/// should not hand every later request on this thread a chunk it will never fill.
const MAX_RETAINED_BYTES: usize = 1 << 20;

impl Default for ResponseArena {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseArena {
    pub fn new() -> Self {
        let bump = ARENA_POOL
            .with(|pool| pool.borrow_mut().pop())
            .unwrap_or_else(|| Box::new(Bump::new()));
        ResponseArena(Some(bump))
    }

    /// A view of the arena at whatever lifetime the caller needs, for allocating values that
    /// will be stored alongside it.
    ///
    /// The lifetime is unbounded, which is the whole point and the whole danger: the caller
    /// has to keep this `ResponseArena` reachable for at least as long as anything allocated
    /// through the returned reference. It exists because the response tree is invariant in its
    /// lifetime — `&'a mut [Value<'a>]` — so a `'static` arena reference could not be narrowed
    /// to the executor's lifetime the way the response `Bytes` used to be.
    #[inline]
    pub fn borrow_unbounded<'a>(&self) -> &'a Bump {
        // SAFETY: see the type-level comment. The reference points into `Bump`'s boxed header
        // and the chunks it owns, neither of which moves for as long as `self` is alive.
        let bump: &Bump = self.0.as_deref().expect("arena is only taken on drop");
        unsafe { &*(bump as *const Bump) }
    }

    /// Capacity of the chunks this arena holds — not the bytes handed out, which `Bump` does
    /// not track. After a reset it is the size of the chunk that was kept.
    pub fn allocated_bytes(&self) -> usize {
        self.0.as_deref().map_or(0, Bump::allocated_bytes)
    }
}

impl Drop for ResponseArena {
    fn drop(&mut self) {
        let Some(mut bump) = self.0.take() else {
            return;
        };
        if bump.allocated_bytes() > MAX_RETAINED_BYTES {
            return;
        }
        bump.reset();
        poison(&mut bump);
        ARENA_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < POOL_CAPACITY {
                pool.push(bump);
            }
        });
    }
}

/// Fills the chunk a reset `Bump` kept with a pattern, so that a value which outlived its
/// arena reads something obviously wrong instead of the next request's data.
///
/// Debug builds only: this is a correctness net for tests, not something to pay for per
/// request in production.
#[cfg(debug_assertions)]
fn poison(bump: &mut Bump) {
    let capacity = bump.chunk_capacity();
    if capacity > 0 {
        bump.alloc_slice_fill_copy(capacity, 0xAAu8);
    }
    bump.reset();
}

#[cfg(not(debug_assertions))]
fn poison(_bump: &mut Bump) {}

impl std::fmt::Debug for ResponseArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResponseArena")
            .field("allocated_bytes", &self.allocated_bytes())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{ResponseArena, MAX_RETAINED_BYTES};

    #[test]
    fn an_arena_comes_back_from_the_pool_empty() {
        // Drain whatever the rest of this thread's tests left behind.
        while super::ARENA_POOL.with(|pool| pool.borrow_mut().pop()).is_some() {}

        let first = ResponseArena::new();
        first.borrow_unbounded().alloc_slice_fill_copy(4096, 1u8);
        assert!(first.allocated_bytes() >= 4096);
        drop(first);
        assert_eq!(super::ARENA_POOL.with(|pool| pool.borrow().len()), 1);

        // Reused rather than reallocated: the chunk survived the round trip, and serving the
        // next tree out of it does not need a new one. That is the whole point of pooling.
        let second = ResponseArena::new();
        assert_eq!(super::ARENA_POOL.with(|pool| pool.borrow().len()), 0);
        let retained = second.allocated_bytes();
        assert!(retained >= 4096, "chunk was freed, not kept: {retained}");
        second.borrow_unbounded().alloc_slice_fill_copy(1024, 2u8);
        assert_eq!(
            second.allocated_bytes(),
            retained,
            "reuse had to allocate a fresh chunk"
        );
    }

    #[test]
    fn an_oversized_arena_is_dropped_rather_than_pooled() {
        while super::ARENA_POOL.with(|pool| pool.borrow_mut().pop()).is_some() {}

        let big = ResponseArena::new();
        big.borrow_unbounded()
            .alloc_slice_fill_copy(MAX_RETAINED_BYTES + 1, 1u8);
        drop(big);
        assert_eq!(super::ARENA_POOL.with(|pool| pool.borrow().len()), 0);
    }
}
