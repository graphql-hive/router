---
hive-router: patch
node-addon: patch
---

# Fix wrong values and invalid subgraph operations from query plan merges

- Fields with different arguments fetched by separate entity calls, for example `price(currency: "USD")` next to a `@requires` on `price(currency: "EUR")` under `@include`, no longer end up under the same response key, so each one gets its own value.
- The same field with different arguments under different type conditions, each with its own `@include`, now makes a valid subgraph operation, and each place in the response gets its own value ([#1311](https://github.com/graphql-hive/router/issues/1311)).
- A `@skip`/`@include` on a field is no longer dropped when a conditional entity call sits next to it.
- Re-entering a subgraph for a `@requires` now uses a key of that subgraph that the earlier fetch actually has, instead of sending another subgraph's key or failing to plan.
