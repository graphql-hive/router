---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Avoid indirect lookup for directly resolved leaf fields

The planner now skips indirect path lookup when a leaf field already has a valid direct path.
