---
hive-router-internal: minor
hive-router: minor
---

# Attach the correlation request-id to the root HTTP server span

The root `http.server` OpenTelemetry span now carries a `router.request_id` attribute, set to the same request-id used for log correlation (either the incoming correlation header, e.g. `x-request-id`, or an auto-generated one). This makes it possible to join a trace directly to its logs without cross-referencing trace IDs.

Fixes https://github.com/graphql-hive/router/pull/1353
