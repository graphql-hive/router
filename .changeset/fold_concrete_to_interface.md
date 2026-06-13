---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Fold repeated object-type selections into a single interface selection

When a `Fetch` node asks for the same fields on different object types, and all
of those types implement the same interface that matches the field's return type,
the query planner now merges them into a single inline fragment on the interface
instead of keeping separate branches.

For example: `query { media { ... on Book { id title } ... on Movie { id title } } }` becomes
`query { media { id title } }` when the field's return type is `Media` and both
`Book` and `Movie` implement it in the subgraph.
