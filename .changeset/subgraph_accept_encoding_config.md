---
hive-router: minor
---

# Configurable `Accept-Encoding` header sent to subgraphs

The router previously always advertised `Accept-Encoding: gzip, deflate, br, zstd` to every
subgraph, with no way to disable it or change the advertised algorithms/order. This is now
configurable under `traffic_shaping.*.compression.response.accept_encoding`:

```yaml
traffic_shaping:
  all:
    compression:
      response:
        accept_encoding:
          publish: true # default true
          algorithms: # order matters - some subgraphs pick the first one they support
            - zstd
            - gzip
            - br
            - deflate
  subgraphs:
    accounts:
      compression:
        response:
          accept_encoding:
            publish: false
```

`publish` controls whether the header is sent at all (default: `true`, matching prior behavior).

`algorithms` controls which encodings are advertised and in what order (default: `gzip, deflate,
br, zstd`, matching the router's previous hard-coded value). An empty `algorithms` list behaves
like `publish: false`.

This only affects what the router asks for - it always transparently decompresses any of `gzip`,
`deflate`, `br`, or `zstd` it receives back from a subgraph, regardless of this setting.

Closes https://github.com/graphql-hive/router/issues/1500
