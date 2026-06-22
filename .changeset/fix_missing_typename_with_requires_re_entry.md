---
hive-router-query-planner: patch
hive-router: patch
node-addon: patch
hive-router-plan-executor: patch
---

# Fix missing `__typename` with `@requires` re-entry

Resolving a field with `@requires` makes the planner re-enter the entity's subgraph through `_entities`, using a representation built from the entity's data. That representation must carry the entity's `__typename`, since `_entities` routes on it.

The fetch that produces the entity now always selects its `__typename`, so the re-entry representation is complete.

Fixes [#1070](https://github.com/graphql-hive/router/issues/1070)
