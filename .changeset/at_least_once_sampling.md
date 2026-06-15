---
hive-router: minor
hive-router-config: minor
hive-console-sdk: minor
hive-router-internal: patch
hive-router-plan-executor: patch
hive-apollo-router-plugin: patch
---

# Add at-least-once sampling for Usage Reporting

Hive Router now supports at-least-once sampling for Usage Reporting.

This feature is useful when you want to keep a low sampling rate, but still make sure all operations are visible in Hive at least once.

The first request for each unique key is always reported. Later requests for the same key follow the configured sampling `rate`.

Example configuration:

```yaml
telemetry:
  hive:
    usage_reporting:
      enabled: true
      sampling:
        rate: "10%" # 10% of operations will be reported
        at_least_once:
          key: # the combination of operation's name and body makes the request unique
            - operation_name
            - operation_body
          max_distinct_keys: 12000 # how many keys to track and hold in memory
```

Keys are tracked in memory, up to `max_distinct_keys` (default: `100_000`). Every key takes approximately 16 bytes of memory.
