---
hive-router-plan-executor: minor
hive-router: patch
---

# Restrictive `Cache-Control` header merging

`Cache-Control` headers from all subgraph responses are now merged using the most conservative directive before being forwarded to the client. No configuration required.

## Merge algorithm

Given N subgraph responses:

1. Any `no-store`, `no-cache`, or `private` directive short-circuits the result to `no-store, no-cache`.
2. Otherwise `max-age` is the minimum of all present values (missing values are ignored).
3. `public` only when every subgraph is `public`.
4. `must-revalidate` if any subgraph sets it.

`no-store, no-cache, must-revalidate` is forced regardless of subgraph headers when:

- Any subgraph executor error (network failure, bad status, etc.)
- Any GraphQL-level error in a subgraph response (`errors` array non-empty)
- The operation is a mutation

The `Cache-Control` header is removed entirely when no subgraph sends a valid one.

## Configuration

Enable merging by propagating `Cache-Control` through header rules:

```yaml
headers:
  all:
    response:
      - propagate:
          named: cache-control
          algorithm: append
          # default emitted unless a subgraph provides one
          default: "public, max-age=180"
```

Subgraph-level `insert` or `remove` rules let you pin or drop a specific subgraph's contribution before the merge runs:

```yaml
headers:
  subgraphs:
    pricing:
      response:
        - insert:
            name: cache-control
            value: "no-cache"
```

Configuring `propagate: named: cache-control` without the merge enabled is rejected at compile time. If no `Cache-Control` propagation is configured, merging is inactive.
