---
hive-router-plan-executor: patch
hive-router-query-planner: patch
hive-router: patch
---

Improve query execution performance by reusing subgraph entity responses within a single query plan execution. When the router needs the same representation fetch more than once, it now reuses the first result instead of sending duplicate subgraph requests.

Query plans now also include deterministic `representationReusePlan` metadata, which lists fetch groups for response reuse.

To make this metadata easier to read, serialized query plans now include `id` on every `Fetch` node and `representationReusePlan.groups` is represented as compact fetch-id arrays.
