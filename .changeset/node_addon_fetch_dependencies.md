---
node-addon: patch
---

# Fetch ids and dependencies in query plans

Every `Fetch` and `BatchFetch` node in the plan now has its `id` and `dependsOn`, the ids of the fetches whose data it reads. A `BatchFetch` also has `completes`, the ids of every fetch merged into it. A fetch can start as soon as the fetches it depends on are done, the way the router schedules them, instead of waiting for its whole `Parallel`/`Sequence` wave.

The plan types also lose `operationName` on `FetchNode` and `BatchFetchNode`, which the plan never had, and gain `customScalarPaths` and the `omit_from_response` flag of required fields, which it did have.
