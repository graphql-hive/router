---
hive-router: minor
hive-router-internal: minor
hive-router-plan-executor: minor
---

# Plugin-provided log correlation identifiers

Plugins can now register a correlation extractor via `register_logger_correlation_extractor` in `on_plugin_init`, to attach custom identifiers (e.g. a tenant or project ID) to every log line produced while handling a request, alongside the built-in `request_id` and `trace_id`.
