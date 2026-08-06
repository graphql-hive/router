---
hive-router: minor
---

# Isolate plugin-selected supergraph configuration

Plugin-selected supergraphs no longer inherit graph-bound settings from the router's configured supergraph. Requests, WebSocket connections, persisted-document resolution, subgraph execution, usage reports, and Hive traces now use the options attached to the selected `Supergraph` snapshot.
