---
hive-router: minor
---

# Add `hive.router.request_dedupe.joined_total` counter metric

Adds observability for the router's inbound request deduplication (`traffic_shaping.router.dedupe`).

| Metric                                      | Labels | Unit        | Description                                                                                     |
| -------------------------------------------- | ------ | ----------- | ------------------------------------------------------------------------------------------------- |
| `hive.router.request_dedupe.joined_total`   | none   | `{request}` | Number of inbound requests that joined an already in-flight deduplicated request instead of executing their own |

The counter increments once per client request (HTTP or WebSocket) that was coalesced into an in-flight leader request. It stays at zero when router-level dedupe is disabled, or when every request executes independently.

Closes https://github.com/graphql-hive/router/issues/1469
