---
hive-router-config: patch
hive-router: patch
hive-router-internal: patch
hive-router-plan-executor: patch
---

# Refactor internal `HiveRouterConfig`

`HiveRouterConfig` can now be treated as `'&static` (by calling `.into_static()`). This makes the work with the config struct easier, as it can be used directly without worrying about lifetimes, and without cloning.
