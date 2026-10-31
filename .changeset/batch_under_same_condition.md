---
hive-router: patch
---

# Batch entity fetches under `@include`/`@skip` conditions

Entity fetches wrapped in `@include` or `@skip` were never batched, each one went out as its own request. They now go into the same batched request as the other entity fetches to that subgraph, whatever their condition, so an operation with many conditional fields sends far fewer subgraph requests.

Each condition gets its own alias, with the same `@include`/`@skip` on the `_entities` field, so the subgraph skips it when it's off. The query plan has the condition on the alias too, as `includeIf` or `skipIf`, and the router doesn't collect or send representations for an alias that's off. Fetches under nested conditions are still grouped by their outer condition and batched inside it.

The executor now also runs every node under a condition nested in a `Parallel`, a shape the grouping produces.
