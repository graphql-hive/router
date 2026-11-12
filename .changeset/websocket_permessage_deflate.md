---
hive-router: minor
---

# WebSocket `permessage-deflate`

The router supports the `permessage-deflate` extension (RFC 7692) on WebSocket connections from clients and to subgraphs. When both sides agree on the extension, messages are compressed in both directions. A side that does not support it declines the extension and the connection continues uncompressed.

It is disabled by default. Compression trades CPU and memory for bandwidth, each compressed connection uses about 420KiB for its compressor and decompressor.

The router accepts the extension from clients with `websocket.permessage_deflate`:

```yaml
websocket:
  enabled: true
  permessage_deflate: true
```

The router offers the extension to WebSocket-enabled subgraphs with `traffic_shaping.all.websocket.permessage_deflate`, and it can be overridden per subgraph:

```yaml
traffic_shaping:
  all:
    websocket:
      permessage_deflate: true
  subgraphs:
    payments:
      websocket:
        permessage_deflate: false
```
