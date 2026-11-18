---
hive-router: minor
---

# Automatic Persisted Queries to subgraphs

The router can send subgraph requests as Automatic Persisted Queries (APQ): only the SHA-256 hash of the query goes over the wire, in `extensions.persistedQuery`. When a subgraph doesn't know the hash yet (`PERSISTED_QUERY_NOT_FOUND`), the router sends the request again with the query, so the subgraph can store it. When a subgraph answers with `PERSISTED_QUERY_NOT_SUPPORTED`, or fails the hash-only request but answers the same query sent in full (a server without APQ, like GraphQL Yoga without the APQ plugin), the router sends the query instead and stops using APQ for that subgraph for 5 minutes, then tries again, so a subgraph that gets APQ later (or an old replica hit during a rollout) doesn't turn it off for good. A hash-only request that fails for another reason (auth, overload) is sent once more as a plain query, and when that fails too, APQ stays on. Mutations skip that extra try, since an error doesn't prove a mutation didn't run, and only retry on `PERSISTED_QUERY_NOT_FOUND` and `PERSISTED_QUERY_NOT_SUPPORTED`. So a subgraph without APQ that only gets mutations needs APQ turned off in the config.

It's off by default. Turn it on for all subgraphs, and override it per subgraph:

```yaml
traffic_shaping:
  all:
    automatic_persisted_queries: true
  subgraphs:
    reviews:
      automatic_persisted_queries: false
```

Only queries and mutations sent over HTTP use APQ, subscriptions don't. A plugin that changes the query should do it in `on_subgraph_execute`, so the hash covers the changed query. A plugin that rewrites the request body in `on_subgraph_http_request` needs APQ off for that subgraph.
