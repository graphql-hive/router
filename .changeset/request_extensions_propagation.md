---
hive-router: minor
---

# Propagate client request `extensions` to subgraphs

The new `request_extensions` configuration sends the `extensions` of the client's GraphQL request to subgraphs. It is off by default. When enabled, the selected extensions are sent with every subgraph request of the operation, including entity fetches and subscriptions.

`mode` decides how `keys` is applied to the top-level extension keys:

- `allow`: propagate only the listed keys.
- `deny`: propagate every key except the listed ones. An empty list propagates every key.

```yaml
request_extensions:
  propagate:
    mode: allow
    keys: [tenant, clientInfo]
```

Keys the router owns are never propagated: `persistedQuery`, `headers`, `subscription`, the JWT claims forwarding field (when `jwt.forward_claims_to_upstream_extensions` is enabled), and the extension keys used by `persisted_documents` selectors (when persisted documents are enabled). The router fails to start when `allow` mode lists one of them, or lists no keys.

Closes https://github.com/graphql-hive/router/issues/1748
