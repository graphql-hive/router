---
hive-router: minor
---

# Health and readiness now pass through the plugin `on_http_request` chain and its `on_end` callbacks

This is required because readiness in plugin-only mode must allow the plugin to select a supergraph for that specific readiness request.

Coprocessors still do not run for health or readiness.
