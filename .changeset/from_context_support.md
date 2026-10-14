---
hive-router: minor
---

# Support Federation `@context` / `@fromContext`

Contextual arguments are resolved per entity from the nearest ancestor that sets the context and sent as `__context_{n}` variables. Entities whose ancestors hold different values are fetched in parallel requests, one per distinct value set. Query plan JSON gets `contextRewrites` on `Fetch` nodes and `BatchFetch` aliases, and a `Parent` path segment.

Fields with a non-null contextual argument are not batched into `BatchFetch` or combined into multi-type fetches. Mutation fields with contextual results finish their context-dependent fetches before the next root field runs.

