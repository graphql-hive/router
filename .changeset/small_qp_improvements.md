---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Query Planning performance improvements

Removed unused per-path edge tracking and switched to references instead of owned values - no cloning of selection items.
