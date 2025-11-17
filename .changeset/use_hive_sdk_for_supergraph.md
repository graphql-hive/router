---
default: minor
---

Use `hive-console-sdk` to load supergraph from Hive Console instead of custom implementation.

### Breaking Changes

The configuration for the `hive` supergraph source has been updated. The `timeout` field is now `request_timeout`, and new options `connect_timeout` and `accept_invalid_certs` have been added.

```yaml
supergraph:
  source: hive
  endpoint: "https://cdn.graphql-hive.com/supergraph"
  key: "YOUR_CDN_KEY"
  # Old `timeout` is now `request_timeout`
  request_timeout: 30s
  # New options
  connect_timeout: 10s
  accept_invalid_certs: false