---
hive-router-internal: minor
hive-router: minor
hive-router-plan-executor: patch
---

# Expose log correlation to the plugin system

Plugins can now attach a custom correlation (e.g. a tenant or project ID) to every log line of the current request via `hive_router::set_log_correlation(key, value)`, callable from any hook, alongside the built-in `request_id` and `trace_id`.

Fixes https://github.com/graphql-hive/router/issues/1350
