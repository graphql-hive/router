---
hive-router: patch
hive-router-plan-executor: patch
---

# Fix response header propagation on error paths

Response header rules now run consistently for successful responses, partial GraphQL error responses, deduped requests, and execution failures.
