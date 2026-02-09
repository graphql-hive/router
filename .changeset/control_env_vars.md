---
hive-router-config: minor
hive-router-internal: patch
hive-router-plan-executor: patch
hive-router: patch
---

Hive telemetry (tracing and usage reporting) is now explicitly opt-in.

Two new environment variables are available to control telemetry:
  - `HIVE_TRACING_ENABLED` controls `telemetry.hive.tracing.enabled` config value
  - `HIVE_USAGE_REPORTING_ENABLED` controls `telemetry.hive.usage_reporting.enabled` config value
  
The accepted values are `true` or `false`.

If you only set `HIVE_ACCESS_TOKEN` and `HIVE_TARGET`, usage reporting stays disabled until explicitly enabled with environment variables or configuration file.
