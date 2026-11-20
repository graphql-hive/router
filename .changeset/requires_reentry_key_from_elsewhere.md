---
hive-router: patch
---

# `@requires` with a key field from another subgraph

A `@requires` field whose subgraph is keyed by a field the entity's original subgraph doesn't have can now be planned. The key is read from a fetch that has it.
