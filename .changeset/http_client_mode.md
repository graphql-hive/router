---
hive-router: minor
---

# `traffic_shaping.http_client_mode` with a per-worker option

Every worker of the router runs its own single-threaded runtime, but all of them shared one HTTP client and connection pool to subgraphs. A connection opened on one worker got picked up by requests on the others, so a subgraph response often had to wake up another thread.

With `http_client_mode: per_worker`, each worker gets its own client, so a request stays on its worker from start to end. In our benchmark, the router used 9.5% less CPU per request. Each worker keeps its own idle connections, so more connections can stay open, while `max_connections_per_host` still caps the requests in flight.

`shared` is the default and keeps one client for all workers.

```yaml
traffic_shaping:
  http_client_mode: per_worker
```
