---
hive-router: patch
---

# `request_timeout` covers reading the subgraph's response body

`traffic_shaping.all.request_timeout` used to stop once the subgraph's response headers arrived. A subgraph that sent its headers and then stalled in the middle of the body kept the request waiting, and held its `max_connections_per_host` slot, until the router's own `request_timeout` failed the whole client request.

The headers and the body now share one deadline, and a stalled body fails with `SUBGRAPH_REQUEST_TIMEOUT`.
