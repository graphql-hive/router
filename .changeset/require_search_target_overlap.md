---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Fixed false circular dependency detection in case of `@requires`

We fixed a query planner bug that could make some valid federated queries fail.

The issue happened when planning fields with nested `@requires` data. The planner compared required selection sets using only the top-level field, ignoring the rest of the selection set. For example, `foo { bar }` and `foo { baz { qux } }` could both be treated as overlapping `foo`.

This could make the planner drop a valid way to fetch the required data too early.
