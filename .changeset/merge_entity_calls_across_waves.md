---
hive-router: patch
node-addon: patch
---

# Fewer subgraph requests for entity calls

Entity calls to the same subgraph that run in different waves now share one request when that doesn't add a wave to the plan. A moved call can start later than before, since a batch waits for everything its calls wait for. With `entity_reuse` on, a call that could copy entities from another one is never merged with it.

Compared with before, 126 plans make fewer subgraph requests, and none make more or take more waves.
