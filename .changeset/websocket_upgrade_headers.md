---
hive-router: minor
---

# WebSocket operations can use headers from the upgrade request

Headers of a WebSocket operation come from the client, either from the `connection_init` payload or from the operation extensions. Headers set by trusted infrastructure on the HTTP upgrade request, like the `X-Real-IP` header set by a reverse proxy, never reached the per-operation stages.

The new `websocket.headers.upgrade` option lists the headers taken from the upgrade request and added to every operation on the connection:

```yaml
websocket:
  enabled: true
  headers:
    upgrade:
      - x-real-ip
```

The listed headers are taken only from the upgrade request. Values with the same name sent by the client in the `connection_init` payload or in the operation extensions are discarded, so the client cannot spoof them. The headers are visible to plugins, coprocessor stages, JWT authentication and expressions, and respect the header propagation rules.

Additionally, the `coprocessor.stages.router.request` documentation now states that for WebSocket connections the hook runs once on the upgrade request, and that the context entries it sets are visible to the per-operation stages of every operation on the connection.
