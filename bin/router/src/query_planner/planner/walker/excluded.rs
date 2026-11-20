use rustc_hash::FxHashSet;

use crate::query_planner::ast::type_aware_selection::TypeAwareSelection;

// TODO: Consider interior mutability with Rc<RefCell<ExcludedFromLookup>> to avoid full clone while traversing
/// Fx sets: they hold a few items, and the walker makes an empty one for nearly every edge it
/// checks, which costs nothing without a random hasher state to set up.
#[derive(Default)]
pub struct ExcludedFromLookup<'graph> {
    pub graph_ids: FxHashSet<&'graph str>,
    pub requirement: FxHashSet<&'graph TypeAwareSelection<'graph>>,
}

impl<'graph> ExcludedFromLookup<'graph> {
    pub fn new() -> ExcludedFromLookup<'graph> {
        Default::default()
    }
}
