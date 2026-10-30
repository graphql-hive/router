---
hive-router: patch
---

# Batch entity fetches under the same `@include`/`@skip` condition

Entity fetches wrapped in `@include` or `@skip` were never batched, each one went out as its own request. Siblings under the same condition are now grouped and batched together, so an operation with many conditional fields sends far fewer subgraph requests.

The executor now also runs every node under a condition nested in a `Parallel`, a shape the grouping produces.
