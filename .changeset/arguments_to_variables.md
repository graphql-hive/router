---
"hive-router": patch
---

Extract hard-coded field argument values into variables, so operations that differ only in those values share one query plan.

`{ user(id: "1") { name } }` and `{ user(id: "2") { name } }` are now planned once, as `query($_v0: ID!) { user(id: $_v0) { name } }`, instead of once per id. That keeps a client looping over IDs from pushing every other plan out of the plan cache. Subgraphs receive the values as variables instead of inline.

It's on by default. A subgraph parses a custom scalar given as a variable with its `parseValue` instead of `parseLiteral`, so turn it off if a subgraph has a scalar where the two behave differently:

```yaml
query_planner:
  arguments_to_variables: false
```
