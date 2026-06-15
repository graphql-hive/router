---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Fix union list FieldMove creation

In some cases union list was treated as single union field in graph. 
