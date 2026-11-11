---
hive-router: minor
---

# WebSocket `permessage-deflate` towards subgraphs

The router can offer the `permessage-deflate` extension (RFC 7692) when connecting to WebSocket-enabled subgraphs. When the subgraph accepts the offer, messages are compressed in both directions. A subgraph that does not support the extension declines the offer and the connection continues uncompressed.

It is disabled by default and can be enabled for all subgraphs or per subgraph:

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
