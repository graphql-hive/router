---
hive-router-internal: minor
hive-router-config: minor
hive-router-plan-executor: minor
hive-router: minor
---

Move `telemetry.hive.endpoint` to `telemetry.hive.tracing.endpoint`.

The endpoint is tracing-specific, but its current placement at `telemetry.hive.endpoint` suggests it applies globally to all Hive telemetry features. This becomes misleading now that usage reporting also defines its own endpoint configuration (`telemetry.hive.usage_reporting.endpoint`).

```diff
telemetry:
  hive:
-   endpoint: "<value>"
+   tracing:
+     endpoint: "<value>"
```
