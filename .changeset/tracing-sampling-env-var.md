---
hive-router: patch
hive-router-config: patch
---

# Add tracing sampling rate environment override

The tracing sampling rate can now be overridden without editing the router config file:

```shell
TRACING_SAMPLING_RATE=0.1
```

This sets the same value as the following YAML configuration:

```yaml
telemetry:
  tracing:
    collect:
      sampling: 0.1
```
