---
hive-router: patch
---

# Send the router's own `Accept` header on subgraph requests

The router now sends `Accept: application/graphql-response+json, application/json;q=0.9` on regular subgraph requests, after header propagation. Before, it sent no `Accept` of its own, so a propagated client `Accept` reached the subgraph unchanged. A client sending `Accept: text/event-stream` made subgraphs that negotiate on `Accept` answer with an event stream, which the router then rejected with `SUBREQUEST_MALFORMED_RESPONSE`. Subscription requests keep their existing `Accept`.
