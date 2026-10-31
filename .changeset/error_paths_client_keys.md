---
hive-router: patch
---

# Error paths point at what the client asked for

A subgraph error inside a field the router fetched for itself, like a `@requires` value under `_internal_qp_alias_0`, or a raw `_entities` path, had a path the client never asked for. Error paths now stop at the closest value the client asked for, and an error with no such value has no path.
