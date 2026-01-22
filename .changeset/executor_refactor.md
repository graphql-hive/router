---
router: patch
executor: patch
---

# Refactor the header propagation logic in `hive-router-plan-executor`

`PlanExecutionOutput` doesn't return `headers: HeaderMap` anymore. Instead, the executor now returns the `ResponseHeaderAggregator` which contains the logic to aggregate headers from subgraph responses. Then, this is applied to the actual response `Response.headers_mut()`.