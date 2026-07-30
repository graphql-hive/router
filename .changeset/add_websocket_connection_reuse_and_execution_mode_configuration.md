---
hive-router-config: minor
hive-router-internal: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Add WebSocket connection reuse and execution mode configuration

WebSocket-enabled subgraphs can now configure connection reuse and choose how queries and mutations are transported.

Configure defaults for all subgraphs under `traffic_shaping.all.websocket`:

```yaml
subscriptions:
  enabled: true
  websocket:
    subgraphs:
      reviews:
        path: /reviews/ws

traffic_shaping:
  all:
    pool_idle_timeout: 50s # default
    websocket:
      reuse_connections: true
      execute_mode: reuse_existing
```

`reuse_connections` defaults to `true`:

- `true` multiplexes matching operations over initialized pooled WebSocket connections
- `false` opens a dedicated connection for each WebSocket operation

`execute_mode` defaults to `http` and supports:

- `http`: queries and mutations always use HTTP
- `reuse_existing`: queries and mutations use an initialized matching WebSocket when available, otherwise they immediately use HTTP
- `websocket`: queries and mutations use WebSocket, creating or joining a pooled connection when reuse is enabled

Settings can be overridden per subgraph. Omitted WebSocket fields inherit the global value:

```yaml
traffic_shaping:
  all:
    pool_idle_timeout: 50s # default
    websocket:
      reuse_connections: true
      execute_mode: reuse_existing

  subgraphs:
    payments:
      pool_idle_timeout: 5s
      websocket:
        reuse_connections: false
        execute_mode: websocket
```

In this example, other WebSocket-enabled subgraphs opportunistically reuse initialized connections. `payments` sends each operation over a dedicated WebSocket.

Pooled WebSockets use the effective `pool_idle_timeout`. A per-subgraph value overrides `traffic_shaping.all.pool_idle_timeout` for both HTTP and WebSocket pools. Active WebSocket operations do not expire.

Connection matching uses the inbound headers selected by `traffic_shaping.router.dedupe.headers`, even when router request deduplication is disabled. Include every header that can affect connection-scoped authentication, authorization, cookies, or tenant identity:

```yaml
traffic_shaping:
  router:
    dedupe:
      headers:
        include: [authorization, cookie, x-tenant]
```
