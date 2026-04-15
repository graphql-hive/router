---
hive-router-plan-executor: minor
hive-router: minor
hive-router-config: minor
hive-router-internal: patch
---

# Federated GraphQL Subscriptions

Hive Router now supports federated GraphQL subscriptions with full protocol coverage across [SSE](https://the-guild.dev/graphql/hive/docs/router/subscriptions/sse), [WebSockets](https://the-guild.dev/graphql/hive/docs/router/subscriptions/websockets), [Multipart HTTP](https://the-guild.dev/graphql/hive/docs/router/subscriptions/multipart-http), [Incremental Delivery](https://the-guild.dev/graphql/hive/docs/router/subscriptions/incremental-delivery), and [HTTP Callback](https://the-guild.dev/graphql/hive/docs/router/subscriptions/http-callback) - for both client-to-router and router-to-subgraph communication. Subscription events spanning multiple subgraphs are resolved automatically: when a subscription field lives in one subgraph but the response includes entity fields owned by others, the router fetches those on every event with no extra configuration.

- [Read the product update](https://the-guild.dev/graphql/hive/product-updates/2026-04-14-hive-router-subscriptions)
- [Subscriptions overview](https://the-guild.dev/graphql/hive/docs/router/subscriptions)
- [Server-Sent Events](https://the-guild.dev/graphql/hive/docs/router/subscriptions/sse)
- [Incremental Delivery over HTTP](https://the-guild.dev/graphql/hive/docs/router/subscriptions/incremental-delivery)
- [Multipart HTTP](https://the-guild.dev/graphql/hive/docs/router/subscriptions/multipart-http)
- [WebSockets](https://the-guild.dev/graphql/hive/docs/router/subscriptions/websockets)
- [HTTP Callback](https://the-guild.dev/graphql/hive/docs/router/subscriptions/http-callback)
