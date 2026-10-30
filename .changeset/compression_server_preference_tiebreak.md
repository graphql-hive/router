---
hive-router: patch
---

# Break response compression ties by the configured `algorithms` order

When a client accepted several algorithms with the same q-value, the router picked whichever the client listed first, so the order of `traffic_shaping.router.compression.response.algorithms` never acted as the documented tie-breaker. Cloudflare, for example, fetches the origin with `Accept-Encoding: gzip, br`, so the router always answered with gzip, even with `br` first in `algorithms`.

The listed order in `Accept-Encoding` carries no preference (RFC 9110 §12.5.3), so equally acceptable algorithms are now ranked by the `algorithms` order. Client q-values still take precedence.

```yaml
traffic_shaping:
  router:
    compression:
      response:
        algorithms:
          - kind: br
          - kind: gzip
```

With this config, `Accept-Encoding: gzip, br` now gets a Brotli response, and a gzip-only client still gets gzip.
