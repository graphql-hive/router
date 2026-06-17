---
hive-router-plan-executor: patch
---

# Log subgraph subscription failures at error level

Subgraph subscription failures (WebSocket handshake, HTTP-callback connect, SSE stream, etc.) are now logged at `error` level via the central `plan.rs` handler, matching how non-subscription subgraph errors are already logged. Previously these failures only reached the client; the router itself logged nothing above `debug`.
