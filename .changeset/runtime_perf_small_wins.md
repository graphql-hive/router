---
"hive-router": patch
---

- Reduce synchronization overhead when cloning supergraph snapshots and generating request IDs.
- Skip external wait tracking when the corresponding telemetry metrics are disabled
- Enable `TCP_NODELAY` for subgraph HTTP connections
- Serialize GraphQL variables directly into request buffers, avoiding intermediate allocations
- Speed up response projection and object lookups with optimized field name comparisons
