use ahash::AHashMap;

type SubgraphName<'a> = &'a str;
type FetchStepHash = u64;
type SubgraphCost = u64;

pub struct SubgraphResponseCostTracker<'a> {
    records: AHashMap<(SubgraphName<'a>, FetchStepHash), SubgraphCost>,
}

impl<'a> Default for SubgraphResponseCostTracker<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SubgraphResponseCostTracker<'a> {
    pub fn new() -> Self {
        Self {
            records: AHashMap::default(),
        }
    }

    pub fn track(
        &mut self,
        subgraph: SubgraphName<'a>,
        fetch_step_hash: FetchStepHash,
        cost: SubgraphCost,
    ) {
        self.records.insert((subgraph, fetch_step_hash), cost);
    }

    pub fn total(&self) -> u64 {
        self.records
            .values()
            .fold(0, |acc, cost| acc.saturating_add(*cost))
    }
}
