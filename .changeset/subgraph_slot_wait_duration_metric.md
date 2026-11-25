---
hive-router: patch
---

# Metric for how long subgraph requests wait on `max_connections_per_host`

When `traffic_shaping.max_connections_per_host` is reached, requests to that subgraph host wait for a free slot, and that wait counts toward `request_timeout`. Nothing showed how long those waits took.

The new `hive.router.subgraph.request.slot_wait.duration` histogram (in seconds) records each wait once it ends. It has the `subgraph.name`, `server.address` and `server.port` labels, and a `result` label for how the wait ended:

- `acquired`: the request got a slot and was sent to the subgraph.
- `timeout`: `request_timeout` ran out first, and the request failed with `SUBGRAPH_REQUEST_SLOT_TIMEOUT`.
- `cancelled`: the request went away while waiting, for example when the client disconnected or the router's own `request_timeout` fired.

Requests that find a free slot don't wait, so they aren't recorded. Long `acquired` waits, or any `timeout` waits, mean the subgraph is slow or `max_connections_per_host` is too low.
