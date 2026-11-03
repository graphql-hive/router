use smallvec::SmallVec;

use crate::query_planner::ast::type_aware_selection::TypeAwareSelection;

/// Subgraphs by their index in `SupergraphState::subgraph_names`, a bit each.
/// Up to 128 subgraphs it's inline, so the copies the search makes don't allocate.
#[derive(Debug, Clone, Default)]
pub struct SubgraphSet(SmallVec<[u64; 2]>);

impl SubgraphSet {
    pub fn insert(&mut self, subgraph_index: u16) {
        let word = usize::from(subgraph_index / 64);
        if self.0.len() <= word {
            self.0.resize(word + 1, 0);
        }
        self.0[word] |= 1 << (subgraph_index % 64);
    }

    pub fn contains(&self, subgraph_index: u16) -> bool {
        self.0
            .get(usize::from(subgraph_index / 64))
            .is_some_and(|word| word & (1 << (subgraph_index % 64)) != 0)
    }
}

/// Requirements already visited, a few at most, so a list beats hashing whole selections.
#[derive(Debug, Clone, Default)]
pub struct VisitedRequirements<'graph>(SmallVec<[&'graph TypeAwareSelection<'graph>; 4]>);

impl<'graph> VisitedRequirements<'graph> {
    pub fn contains(&self, requirement: &TypeAwareSelection<'graph>) -> bool {
        self.0
            .iter()
            .any(|visited| std::ptr::eq(*visited, requirement) || *visited == requirement)
    }

    pub fn insert(&mut self, requirement: &'graph TypeAwareSelection<'graph>) {
        if !self.contains(requirement) {
            self.0.push(requirement);
        }
    }
}

/// What a search for indirect paths shouldn't go through again.
#[derive(Debug, Clone, Default)]
pub struct ExcludedFromLookup<'graph> {
    pub subgraphs: SubgraphSet,
    pub requirements: VisitedRequirements<'graph>,
}

impl<'graph> ExcludedFromLookup<'graph> {
    pub fn new() -> ExcludedFromLookup<'graph> {
        Default::default()
    }

    /// The same, with `subgraph_index` excluded too.
    pub fn with_subgraph(&self, subgraph_index: u16) -> Self {
        let mut excluded = self.clone();
        excluded.subgraphs.insert(subgraph_index);
        excluded
    }
}

#[cfg(test)]
mod tests {
    use super::SubgraphSet;

    #[test]
    fn subgraph_set_spills_past_inline_words() {
        let mut set = SubgraphSet::default();
        for index in [0, 63, 64, 127, 128, 700] {
            assert!(!set.contains(index));
            set.insert(index);
            assert!(set.contains(index));
        }
        assert!(!set.contains(1) && !set.contains(129) && !set.contains(701));
    }
}
