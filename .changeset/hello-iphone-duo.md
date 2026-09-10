---
hive-router: minor
---

# BREAKING: Make plugin query plans read-only

Plugins can still inspect generated query plans in `on_query_plan`, but can no longer replace the plan that the router executes.
