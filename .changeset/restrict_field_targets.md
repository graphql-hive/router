---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Restrict indirect path finding to valid subgraphs

Fix an issue where query planning could appear stuck on some complex federated schemas due to excessive indirect path exploration.

Indirect path exploration happens when the planner cannot resolve a field directly in the current subgraph and starts searching for a route through other subgraphs that could satisfy the field and its requirements.

Indirect field lookup is now limited to subgraphs that can actually resolve the requested field, reducing unnecessary work and preventing planning stalls.
