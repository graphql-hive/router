---
hive-router: minor
hive-console-sdk: minor
hive-router-internal: patch
hive-router-plan-executor: patch
hive-apollo-router-plugin: patch
---

# Apply usage-reporting excludes before sampling

Exclusion of Usage Reports is now evaluated before sampling. Excluded operations are dropped immediately and sampling is not affected.
