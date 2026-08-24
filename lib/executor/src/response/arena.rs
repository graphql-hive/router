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
#[derive(Default)]
pub struct ResponseArena(Box<Bump>);

impl ResponseArena {
    pub fn new() -> Self {
        ResponseArena(Box::new(Bump::new()))
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
        unsafe { &*(&*self.0 as *const Bump) }
    }

    /// Bytes currently held, for diagnostics.
    pub fn allocated_bytes(&self) -> usize {
        self.0.allocated_bytes()
    }
}

impl std::fmt::Debug for ResponseArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResponseArena")
            .field("allocated_bytes", &self.allocated_bytes())
            .finish()
    }
}
