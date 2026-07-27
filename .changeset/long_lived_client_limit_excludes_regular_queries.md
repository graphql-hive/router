---
hive-router: patch
hive-router-config: patch
hive-router-internal: patch
hive-router-plan-executor: patch
---

# Regular queries no longer count toward `max_long_lived_clients`

The long-lived client limit (`traffic_shaping.router.max_long_lived_clients`) classified requests from the `Accept` header alone, so clients that advertise streaming support on every operation (e.g. urql sends `..., text/event-stream, multipart/mixed`) had all of their queries and mutations counted against — and, past the limit, rejected by — the limit.

Long-lived clients are now counted where they actually become known:

- **WebSocket connections** are still reserved at upgrade time.
- **HTTP subscriptions** reserve a slot once the parsed operation is known to be a subscription — still before any planning/execution work — and release it when the client stream ends.

Regular queries and mutations never count toward the limit, regardless of their `Accept` header, matching the documented behavior. Over-limit rejections keep the same response (`503`, `Retry-After: 5`, `Too many long-lived clients`) and now happen inside the request span, so they are visible to tracing.
