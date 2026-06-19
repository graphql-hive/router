---
hive-router-config: minor
hive-router-internal: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Make the subscription subgraph executor buffer capacity configurable

When a subscription is established, the router reads events from the subgraph (over HTTP streaming or WebSocket) and runs each one through entity resolution before fanning it out to listeners. A per-subscription buffer sits between the subgraph and that processing pipeline so the subgraph is never throttled when the router falls behind. When the buffer is full, the newest incoming event is dropped (and logged) instead of slowing down or tearing down the connection to the subgraph.

The size of this buffer is now configurable via `subscriptions.subgraph_buffer_capacity`. A larger capacity gives the router more headroom to absorb bursts at the cost of memory and potentially staler events under sustained backpressure; a smaller capacity keeps memory minimal and drops eagerly. It defaults to `1024`, favoring high throughput.

```yaml
subscriptions:
  enabled: true
  subgraph_buffer_capacity: 1024 # default
```
