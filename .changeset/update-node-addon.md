---
node-addon: patch
---

This patch includes the fixes in the query planner including the fixes for mismatch handling so conflicting fields are tracked by response key (alias-aware), and internal alias rewrites restore the original client-facing key (alias-or-name) instead of always the schema field name.