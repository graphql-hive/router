---
hive-router-plan-executor: patch
hive-router: patch
---

# Decouple HTTP streaming subscriptions from downstream backpressure

When a subscription's events flow through the router, each event is run through entity resolution (fetching the related data from other subgraphs) before being delivered to the client. If that resolution is slow, or the client is slow to consume, the router would previously stop reading from the subscribing subgraph until it caught up. That stall propagates back over the connection and effectively throttles the subgraph's emitter.

HTTP streaming subscriptions (multipart and SSE) now buffer incoming events and drain them from the subgraph at full speed, independent of how fast the router can process them. If the router cannot keep up, the oldest buffered event is dropped (and logged) instead of slowing the subgraph.

The subscription stays alive and the subgraph keeps emitting unaffected.
