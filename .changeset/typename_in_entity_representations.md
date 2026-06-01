---
hive-router-plan-executor: patch
hive-router: patch
---

## Always include `__typename` in `_entities` representations

When resolving a field via `@requires`, the executor now resolves `__typename` in the entity representation using the enclosing inline fragment's type condition, even when the parent's response data did not have `__typename`. Previously, the receiving subgraph could fail with `tried to load an entity for type "undefined"` whenever the client query did not explicitly select `__typename` on the parent entity.
