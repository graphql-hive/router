---
node-addon: minor
hive-router-query-planner: minor
hive-router-plan-executor: patch
hive-router: patch
---

## Improve Query Plans for abtract types

The query planner now combines fetches for multiple matching types into a single fetch step.
Before, the planner could create one fetch per type.
Now, it can fetch many types together when possible, which reduces duplicate fetches and makes query plans more efficient.
