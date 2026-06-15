---
hive-router-config: minor
hive-router-internal: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Move `sample_rate` into `sampling.rate`

**Breaking change** The sampling configuration of Usage Reporting has been reorganized.

```diff
telemetry:
  hive:
    usage_reporting:
-      sample_rate: "10%"
+      sampling:
+        rate: "10%"
```


The old top-level `sample_rate` field has been replaced by `sampling.rate`.
