---
hive-router: minor
---

# Isolate plugin-selected supergraph configuration

Plugin-selected supergraphs no longer inherit graph-bound settings from the router's configured supergraph. Requests, WebSocket connections, persisted-document resolution, subgraph execution, usage reports, and Hive traces now use the options attached to the selected `Supergraph` snapshot.

Persisted-document reloaders and Hive usage agents now use separate background-task groups scoped to the selected supergraph runtime. Cancelling a runtime waits for any active Hive flush and explicitly flushes the remaining report buffer before removing its worker. Router shutdown also waits for these graceful background tasks to finish.
