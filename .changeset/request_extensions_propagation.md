---
hive-router: minor
---

# Propagate client request `extensions` to subgraphs

The new `request_extensions` configuration sends the `extensions` of the client's GraphQL request to subgraphs. It is off by default. When enabled, the selected extensions are sent with every subgraph request of the operation, including entity fetches and subscriptions.

Rules can be set for all subgraphs (`propagate.all`) and for individual subgraphs (`propagate.subgraphs.<name>`). A subgraph rule replaces the `all` rule for that subgraph, and a subgraph set to `null` receives no extensions.

`mode` decides how `keys` is applied to the top-level extension keys:

- `allow`: propagate only the listed keys.
- `deny`: propagate every key except the listed ones. An empty list propagates every key.

```yaml
request_extensions:
  propagate:
    all:
      mode: allow
      keys: [tenant]
    subgraphs:
      reviews:
        mode: allow
        keys: [tenant, clientInfo]
      inventory: null # receives no extensions
```

Keys the router owns are never propagated: `persistedQuery`, `headers`, `subscription`, the JWT claims forwarding field (when `jwt.forward_claims_to_upstream_extensions` is enabled), and the extension keys used by `persisted_documents` selectors (when persisted documents are enabled). The router fails to start when an `allow` rule lists one of them, or lists no keys.

Closes https://github.com/graphql-hive/router/issues/1748
