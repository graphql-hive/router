---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Preserve client aliases in mismatch rewrites

Fixed query planner mismatch handling so conflicting fields are tracked by response key (alias-aware), and internal alias rewrites restore the original client-facing key (alias-or-name) instead of always the schema field name.
