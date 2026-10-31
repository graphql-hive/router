---
hive-router: patch
---

# Batch entity fetches under `@include`/`@skip` conditions

Entity fetches wrapped in `@include` or `@skip` were never batched, each one went out as its own request. They now go into the same batched request as the other entity fetches to that subgraph, whatever their condition, so an operation with many conditional fields sends far fewer subgraph requests.

Each set of conditions gets its own alias, nested ones included. The query plan has the conditions on the alias, as `includeIf` and `skipIf` lists, and the router doesn't collect representations for an alias that's off, it sends `[]` for it. With every alias off there's no request.

The executor now also runs every node under a condition nested in a `Parallel`.
