---
hive-router-plan-executor: patch
hive-router-query-planner: patch
hive-router: patch
node-addon: patch
---

# Preserve custom scalars as raw JSON

Custom scalar fields marked by the query planner are now preserved as raw JSON instead of being parsed and rebuilt as structured response values. This improves correctness for JSON passthrough custom scalars while avoiding performance regressions for normal response handling.
