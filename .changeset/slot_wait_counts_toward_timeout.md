---
hive-router: patch
---

# Waiting for a subgraph slot counts toward the request timeout

`traffic_shaping.max_connections_per_host` limits how many requests to one subgraph URL can be in flight at once. Until now, a request that waited for a free slot only started its `request_timeout` once it got one. If one subgraph instance hung, requests to it held their slots until they timed out. Once every slot was taken, all other requests to that subgraph queued with no limit, including ones that would have gone to healthy instances.

The time spent waiting for a slot now counts toward `request_timeout`. A request that can't get a slot in time fails with the new `SUBGRAPH_REQUEST_SLOT_TIMEOUT` error code instead of waiting indefinitely. If you see it, the subgraph is slow or `max_connections_per_host` is too low for your traffic.

These requests never reached the subgraph, so the circuit breaker ignores them: they count neither as failures nor as successes, and they don't count as probes while the breaker is half-open.
