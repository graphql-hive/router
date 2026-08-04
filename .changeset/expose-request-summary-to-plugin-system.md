---
hive-router-internal: minor
hive-router: minor
hive-router-plan-executor: patch
---

# Expose the request summary to the plugin system

Plugins can now enrich the request summary log line with custom attributes via `hive_router::set_summary_attribute(key, value)`, callable from any hook (e.g. `on_http_request`, `on_graphql_analysis`).

Fixes https://github.com/graphql-hive/router/issues/1368
