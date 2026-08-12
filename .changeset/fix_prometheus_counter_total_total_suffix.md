---
hive-router: patch
---

# Fix doubled `_total` suffix on Prometheus counters

The built-in Prometheus metrics exporter (`/metrics`) generated counter names with a
doubled suffix, e.g. `hive_router_graphql_errors_total_total` instead of
`hive_router_graphql_errors_total`.

Counter names on `/metrics` now end in a single `_total`, matching standard
Prometheus conventions.

OTLP metrics exporter is not affected.
