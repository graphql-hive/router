---
hive-router-internal: patch
hive-router-config: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Logger improvements and access logs

Reworked the router's logging for lower overhead and clearer output.

- **Access logs:** at the default `info` level the router now emits a single per-request summary (`router::request` target) with operation, subgraph, error, status, payload size, and duration fields.
- **Correlation:** every log line carries `request_id` (from the `log.correlation.id_header`, default `x-request-id`, or generated when absent) and `trace_id` (from the W3C `traceparent` context when `log.correlation.trace_propagation` is enabled).
- **Explicit targets:** all logs use `router::*` targets, so `log.filter` (or `LOG_FILTER`) can raise or mute individual subsystems. To disable access logs entirely, set `LOG_FILTER=router::request=off`.
- **Internal crates:** logs from dependencies like `ntex` and `hyper` are now suppressed unless `log.log_internals` (or `LOG_INTERNALS`) is enabled.
- **Structured output:** flat JSON/text with no nested fields, formatted directly into buffers.

**Breaking changes:**

- The `trace` log level is no longer available in release builds.
- The `pretty-tree` and `pretty-compact` log formats were removed; only `text` and `json` remain.
