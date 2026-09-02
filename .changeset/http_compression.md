---
hive-router: minor
---

# HTTP compression between the client and the router

Adds `traffic_shaping.router.compression`, controlling response compression (router → client)
and request decompression (client → router). 

`gzip`, `deflate`, `br` (Brotli), and `zstd` are all supported in both directions.

The following defaults are set: 

```yaml
traffic_shaping:
  router:
    compression:
      response:
        enabled: true
        algorithms:
          - kind: gzip
          - kind: zstd
            level: 3
          - kind: br
            quality: 5
          - kind: deflate
        min_size: 1KiB
      request:
        enabled: true
        algorithms: [gzip, zstd, br, deflate]
```

Response compression is negotiated against the client's `Accept-Encoding`.

Request decompression is applied based on the client's `Content-Encoding`.

Both directions default to enabled, with `gzip`, `zstd`, `br`, and `deflate` all allowed.

Closes https://github.com/graphql-hive/router/issues/315
