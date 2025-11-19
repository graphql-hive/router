---
router: minor
---

# Use `hive-console-sdk` to load supergraph from Hive CDN

**Breaking Changes**

The configuration for the `hive` source has been updated to offer more specific timeout controls and support for self-signed certificates. The `timeout` field has been renamed.

```diff
supergraph:
  source: hive
  endpoint: "https://cdn.graphql-hive.com/supergraph"
  key: "YOUR_CDN_KEY"
- timeout: 30s
+ request_timeout: 30s          # `request_timeout` replaces `timeout`
+ connect_timeout: 10s          # new option to control `connect` phase
+ accept_invalid_certs: false   # new option to allow accepting invalid TLS certificates
```
