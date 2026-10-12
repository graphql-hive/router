---
hive-router: minor
---

# Support Federation `@context` / `@fromContext`

Contextual arguments are resolved per entity from the nearest ancestor and sent as `__context_{n}` variables. New `contextRewrites` on `Fetch` and `BatchFetch` aliases and a `Parent` path segment in the query plan JSON.
