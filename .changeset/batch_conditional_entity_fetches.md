---
hive-router: minor
node-addon: minor
---

# Batch conditional entity fetches with their siblings

Entity fetches under `@include`/`@skip` now share a `BatchFetch` with the other fetches to the same subgraph, whatever their conditions. The router only collects representations for the paths whose conditions hold, and skips a request when nothing in it would run. Root and entity fetches whose fields are all skipped don't send a request at all.

This changes the query plan format:

- `EntityBatchAlias.paths` is replaced by `pathGroups`, each with an optional `runIf` and its `paths`.
- `EntityBatchAlias.runVariableName` names a `Boolean!` variable the executor has to send, the alias has `@include(if: $<name>)` on it.
- `FetchNode.runIf` lists when the fetch runs, missing means always.

Anything else that executes these plans has to support the new fields.

Demand control costs each alias under its own conditions.
