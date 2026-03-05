---
hive-router-internal: patch
hive-router: patch
hive-router-plan-executor: patch
---

# Adds `noop_otlp_exporter` feature for internal usage

Hive Router uses `noop_otlp_exporter` internally for testing purposes. This change adds the `noop_otlp_exporter` feature to the `hive-router` crate so that it can be used internally while testing the router.
