---
hive-router: minor
---

# Refresh the remote JWKS when a token's `kid` is unknown

A token signed with a freshly rotated key was rejected until the next `polling_interval` tick. With `refresh_on_unknown_kid` set on a remote JWKS provider, an unknown `kid` re-fetches the JWKS right away. The value is the minimum time between two such re-fetches, so concurrent misses share one fetch and arbitrary `kid`s cannot hammer the endpoint. Disabled by default.

```yaml
jwt:
  jwks_providers:
    - source: remote
      url: https://auth.example.com/.well-known/jwks.json
      refresh_on_unknown_kid: 30s
```
