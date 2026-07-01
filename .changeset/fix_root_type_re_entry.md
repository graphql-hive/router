---
hive-router: patch
hive-router-query-planner: patch
hive-router-plan-executor: patch
node-addon: patch
---

# Fix root type re-entry

When a field re-exposes a root type from a nested position (e.g. a mutation field
returning a type with a `query: Query` field), the query planner could not resolve
selections that live in a different subgraph, and the executor merged whatever it
did fetch at the response root instead of the nested path — so those fields
resolved to `null`, or failed to resolve fully. 

Fixes [#1164](https://github.com/graphql-hive/router/issues/1164)
