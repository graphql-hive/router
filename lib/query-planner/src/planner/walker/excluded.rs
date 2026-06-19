use std::collections::HashSet;

use crate::ast::type_aware_selection::TypeAwareSelection;

// TODO: Consider interior mutability with Rc<RefCell<ExcludedFromLookup>> to avoid full clone while traversing
#[derive(Default)]
pub struct ExcludedFromLookup<'graph> {
    pub graph_ids: HashSet<&'graph str>,
    pub requirement: HashSet<TypeAwareSelection<'graph>>,
}

impl<'graph> ExcludedFromLookup<'graph> {
    pub fn new() -> ExcludedFromLookup<'graph> {
        Default::default()
    }
}
