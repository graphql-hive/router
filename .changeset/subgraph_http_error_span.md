---
hive-router-internal: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Mark failed subgraph HTTP requests as errors on their trace span

When an outgoing subgraph HTTP request failed at the transport level (connection error, timeout, body read failure, etc.), the `http.client` span was left with an unset `otel.status_code`, so the failure was not surfaced as an error in traces (e.g. Datadog). The error was only recorded in metrics. The span is now marked with `otel.status_code = "Error"` and the corresponding `error.type` on the failure path, matching the existing metrics behaviour.
