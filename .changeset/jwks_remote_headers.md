---
hive-router: minor
---

# Add `headers` to remote JWKS providers

`jwt.jwks_providers` with `source: remote` now accepts a `headers` map, applied to the JWKS fetch.

```yaml
jwt:
  jwks_providers:
    - source: remote
      url: http://idp.identity.svc.cluster.local/oauth/v2/keys
      headers:
        Host: auth.example.com
      polling_interval: "15m"
```

This makes two previously unreachable setups work.

An identity provider that resolves its tenant or instance from the `Host` header cannot be addressed by an internal URL: the connection succeeds, but the provider sees the internal authority, finds no tenant registered for it, and the fetch fails. Zitadel is one such provider — it matches the request `Host` against its registered instance domains and otherwise answers `Instance not found`. Setting `Host` here lets the router reach the provider over a cluster-internal address while still identifying the tenant, instead of routing every key refresh out through a public load balancer and back.

A JWKS endpoint behind a gateway that expects an API key or a custom auth header is now also reachable, without a proxy in front of the router.

Headers are applied to the request only; they do not change where the request is sent. The field defaults to empty, so existing configurations are unaffected.

Closes https://github.com/graphql-hive/router/issues/1475
