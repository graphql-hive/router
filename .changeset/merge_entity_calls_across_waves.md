---
hive-router: patch
node-addon: patch
---

# Fewer subgraph requests for entity calls

Entity calls to the same subgraph that run in different waves now share one request when that doesn't make the plan longer. Calls move to the earliest wave they can all run in, so nothing starts later than it has to. With `entity_reuse` on, a call that could copy entities from another one is never merged with it.

Compared with before, 126 plans make fewer subgraph requests, and none make more or take more waves.
