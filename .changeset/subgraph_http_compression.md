---
hive-router: minor
---

# HTTP compression between the router and subgraphs

Implements HTTP compression for router ↔ subgraph calls. `gzip`, `deflate`, `br` (Brotli), and `zstd` are supported.

```yaml
traffic_shaping:
  all:
    compression:
      request:
        enabled: false
        algorithm:
          kind: gzip
  subgraphs:
    accounts:
      compression:
        request:
          enabled: true
          algorithm:
            kind: zstd
            level: 5
```

Compressing outbound requests to a subgraph is opt-in and off by default, since compressing
unconditionally could break a subgraph that doesn't decompress. 

A per-subgraph override fully replaces the `all` default rather than merging field-by-field.

Decompressing subgraph responses is always on and unconfigured, so the router transparently decompresses any subgraph response carrying a
recognized `Content-Encoding` regardless of the outbound setting.

The router now also always advertises `Accept-Encoding: gzip, deflate, br, zstd` to every subgraph, independent of whether outbound compression is enabled.

Closes https://github.com/graphql-hive/router/issues/315
