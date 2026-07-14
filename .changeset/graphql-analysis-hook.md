---
hive-router-plan-executor: minor
hive-router: patch
---

## Add an `on_graphql_analysis` plugin hook with safe operation filtering

Plugin authors can now inspect and filter operation fields after normalization and immediately before query planning. Fields can be kept or nulled with a GraphQL error while the router consistently updates the operation and response projection plan, making the hook suitable for authorization, rate limiting, progressive overrides, and similar policies.
