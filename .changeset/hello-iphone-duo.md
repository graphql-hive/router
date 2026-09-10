---
hive-router: minor
---

# BREAKING: Make query plans read-only in plugins

Plugins can still inspect generated query plans in `on_query_plan`, but can no longer replace the plan that the router executes.
