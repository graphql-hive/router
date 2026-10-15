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
        // the hash identifies the operation text, not an individual execution. fetches with
        // different requires can have identical operations, and alias limits can split them
        // into separate requests. context groups can also execute the same operation repeatedly.
        // every response contributes to actual cost, so accumulate even when the hash matches.
        // saturate so large totals can't overflow and undercount the cost
        let total = self.records.entry((subgraph, fetch_step_hash)).or_default();
        *total = total.saturating_add(cost);
    }

    pub fn total(&self) -> u64 {
        self.records
            .values()
            .fold(0, |acc, cost| acc.saturating_add(*cost))
    }
}
