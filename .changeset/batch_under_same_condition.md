---
hive-router: patch
---

# Batch conditional entity fetches

Entity fetches under `@include`/`@skip` used to go out as separate requests. They're now batched with the other entity fetches to the same subgraph, so operations with many conditional fields send far fewer requests.

A fetch whose condition is false sends no entities, and if every fetch in the batch is off, no request is sent. Demand control only counts the fetches that run.
