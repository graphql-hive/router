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
      headers: # new 
        Host: auth.example.com
      polling_interval: "15m"
```

A JWKS endpoint behind a gateway that expects an API key or a custom auth header is now also reachable, without a proxy in front of the router.

Closes https://github.com/graphql-hive/router/issues/1475
