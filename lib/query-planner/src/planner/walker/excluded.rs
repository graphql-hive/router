use crate::ast::type_aware_selection::TypeAwareSelection;
use hashbrown::HashSet;

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

    pub fn next(
        &self,
        graph_id: &str,
        requirements: &HashSet<TypeAwareSelection>,
    ) -> ExcludedFromLookup {
        let mut graph_ids = self.graph_ids.clone();
        graph_ids.insert(graph_id.to_string());

        ExcludedFromLookup {
            graph_ids,
            requirement: requirements.clone(),
        }
    }
}
