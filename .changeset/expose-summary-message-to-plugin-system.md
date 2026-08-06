---
hive-router-internal: minor
hive-router: minor
hive-router-plan-executor: patch
---

# Expose the summary message to the plugin system

Plugins can now override the request summary log line's message via `hive_router::set_summary_message(message)`, callable from any hook.

Fixes https://github.com/graphql-hive/router/issues/1378
