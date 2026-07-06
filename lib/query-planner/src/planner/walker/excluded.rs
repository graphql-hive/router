use std::collections::HashSet;

#[derive(Default)]
pub struct ExcludedFromLookup<'graph> {
    pub graph_ids: HashSet<&'graph str>,
}

impl<'graph> ExcludedFromLookup<'graph> {
    pub fn new() -> ExcludedFromLookup<'graph> {
        Default::default()
    }
}
