---
hive-router: patch
---

# Waiting for a subgraph slot counts toward the request timeout

`request_timeout` used to start only once a request got a slot under `traffic_shaping.max_connections_per_host`. When one subgraph instance hung and its requests took every slot, all other requests to that subgraph waited until the router's own `traffic_shaping.router.request_timeout` (60s by default) failed the whole client request, with no data from the other subgraphs either.

Now the wait counts toward `request_timeout`, and a request that can't get a slot in time fails with the new `SUBGRAPH_REQUEST_SLOT_TIMEOUT` error code, and the router can still return data from the other subgraphs. Seeing it means the subgraph is slow or `max_connections_per_host` is too low. The circuit breaker ignores these requests, since they never reached the subgraph.

The same budget covers deduplicated requests: a request waiting on an identical one in flight gives up at its own timeout, and if it takes over after that one fails, it only gets the time it has left.
