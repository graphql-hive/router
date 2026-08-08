---
hive-router: minor
---

# Multiplex and reuse WebSocket subgraph connections

The router can now multiplex GraphQL operations over shared `graphql-transport-ws` subgraph connections.

Subscriptions with the same subgraph and inbound connection identity reuse one initialized connection instead of opening one WebSocket per operation. Different operations retain independent streams while sharing the physical connection.

Queries and mutations can also reuse a connection opened by a subscription:

```yaml
subscriptions:
  enabled: true
  websocket:
    subgraphs:
      reviews:
        path: /reviews/ws

traffic_shaping:
  all:
    websocket:
      reuse_connections: true
      execute_mode: reuse_existing
```

With this configuration:

1. A subscription initializes the pooled `reviews` connection.
2. Matching subscriptions multiplex over it.
3. Matching queries and mutations use it while it remains initialized.
4. A query or mutation uses HTTP when the connection is missing, expired, or still initializing.

Use WebSocket for every operation by selecting `websocket` mode:

```yaml
traffic_shaping:
  all:
    websocket:
      reuse_connections: true
      execute_mode: websocket
```

The first operation initializes the connection, concurrent operations join that initialization, and later matching operations reuse the initialized connection.

Once an operation selects WebSocket, transport failures and timeouts are returned to the client without retrying over HTTP. This prevents a mutation that may have reached the subgraph from being executed twice.

Idle pooled connections close after the effective `pool_idle_timeout`. Active operations keep the connection open, and dropping one operation cancels only that operation without closing the shared connection.

The router also exposes WebSocket pool telemetry for active connections and operations, initialization success and failure, initialization waiters, reuse lookup hits and misses, and connection closure reasons. These metrics help measure reuse hit rate, multiplexing, connection churn, handshake failures, and per-subgraph pool usage.
