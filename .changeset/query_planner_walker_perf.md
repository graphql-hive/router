---
hive-router: patch
---

# Speed up the query planner walker and fetch graph

- Look for indirect paths only when there's an entity or interface-object move to follow, and work out the target subgraphs only when that lookup runs.
- Share path lists between work items instead of cloning them, and follow inline fragments with a single direct path.
- Merge leaves and siblings faster, and price a merged plan by measuring only the nodes the merge adds.
- Group fetch steps by a key before batching multi-type fetches, so only steps that could be batched get compared.
