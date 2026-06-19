---
hive-router-plan-executor: patch
hive-router: patch
---

# Keep WebSocket subgraph subscriptions alive under backpressure

Each subscription event the router receives is run through entity resolution (fetching related data from other subgraphs) before reaching the client. When that resolution has higher latency than the rate at which the subgraph emits events, the router falls behind and backpressure builds up.

The WebSocket subgraph executor now drops individual messages it cannot keep up with instead of tearing down the subscription, keeping the underlying connection to the subgraph open. The dropped messages are logged, and the subgraph continues emitting without being throttled by the router's processing speed.
