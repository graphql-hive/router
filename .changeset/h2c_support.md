---
hive-router-config: patch
hive-router-plan-executor: patch
hive-router: patch
hive-router-internal: patch
---

# HTTP/2 Cleartext (h2c) Support for Subgraph Connections

Adds support for HTTP/2 cleartext (h2c) connections between the router and subgraphs via the new `allow_only_http2` configuration flag. When enabled, the router uses HTTP/2 prior knowledge to communicate with subgraphs over plain HTTP without TLS.

This is useful in environments where subgraphs support HTTP/2 but TLS is not required, such as service meshes, internal networks, or sidecar proxies.

## Configuration

The flag can be set globally for all subgraphs or per-subgraph. Per-subgraph settings override the global default.

### Global (all subgraphs)

```yaml
traffic_shaping:
  all:
    allow_only_http2: true
```

### Per-subgraph

```yaml
traffic_shaping:
  subgraphs:
    accounts:
      allow_only_http2: true
```

The default value is `false`, preserving the existing behavior of using HTTP/1.1 for plain HTTP connections and negotiating HTTP/2 via ALPN for TLS connections.
