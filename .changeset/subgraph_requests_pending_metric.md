---
hive-router: patch
---

# Metric for subgraph requests waiting on `max_connections_per_host`

`traffic_shaping.max_connections_per_host` caps how many requests the router sends at once to one subgraph host. When that limit was reached, further requests waited for a free slot, and nothing showed how many were waiting.

The new `hive.router.subgraph.requests.pending` metric counts the subgraph requests that are waiting for a slot right now. It has the `subgraph.name`, `server.address` and `server.port` labels. Subgraphs served from the same host share the limit, so add up their series to see that host's queue.

A request counts while it waits, and stops counting once it gets a slot, times out (`SUBGRAPH_REQUEST_SLOT_TIMEOUT`) or is cancelled. Requests that find a free slot are never counted. A value that stays above `0` means the subgraph is slow or `max_connections_per_host` is too low.

Closes https://github.com/graphql-hive/router/issues/1958
