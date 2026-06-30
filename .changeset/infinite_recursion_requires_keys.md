---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Prevent @requires creating a circular dependency across subgraphs

The Query Planner could hit a timeout when a field with `@requires` needed to move to another subgraph, and the move required the same fields.
