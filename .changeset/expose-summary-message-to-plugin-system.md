---
hive-router-internal: minor
hive-router: minor
hive-router-plan-executor: patch
---

# Expose the summary message to the plugin system

Plugins can now override the request summary log line's message via `hive_router::set_summary_message(message)`, callable from any hook. The first call wins for a given request; if never called, the summary line keeps its default (no message), as before.
