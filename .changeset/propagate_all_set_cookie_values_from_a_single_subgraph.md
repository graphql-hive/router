---
hive-router-plan-executor: patch
hive-router: patch
---

# Propagate all multi-instance headers from a single subgraph response

When a subgraph responded with multiple instances of a never-join header (`Set-Cookie` or `WWW-Authenticate`), the router only forwarded one of them to the client and silently dropped the rest.

The fix is to propagate all values of a never-join header as separate header fields end-to-end, rather than just the first value.

Fixes https://github.com/graphql-hive/router/issues/1388
