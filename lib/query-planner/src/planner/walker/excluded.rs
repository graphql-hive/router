use std::collections::HashSet;

use crate::ast::type_aware_selection::TypeAwareSelection;

// TODO: Consider interior mutability with Rc<RefCell<ExcludedFromLookup>> to avoid full clone while traversing
#[derive(Default)]
pub struct ExcludedFromLookup {
    pub graph_ids: HashSet<String>,
    pub requirement: HashSet<TypeAwareSelection>,
}

impl ExcludedFromLookup {
    pub fn new() -> ExcludedFromLookup {
        Default::default()
    }
}
