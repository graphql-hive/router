---
hive-router: patch
---

# Batch entity fetches under `@include`/`@skip` conditions

Entity fetches with a `runIf` (from `@include`/`@skip` around them or on their fields) were never batched, each one went out as its own request. They now go into the same batched request as the other entity fetches to that subgraph, whatever their condition, so an operation with many conditional fields sends far fewer subgraph requests.

Each condition gets its own alias, with the fetch's `runIf` on it. The router doesn't collect representations for an alias that's off, it sends `[]` for it. With every alias off there's no request. Demand control only counts the cost of the aliases that are on.
