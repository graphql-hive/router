---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Improve handling of unions

The query planner improves handling of union types whose members vary between subgraphs. Previously, the planner always computed an intersection of union members, ignoreing subgraph-specific members.

Fixes [#1098](https://github.com/graphql-hive/router/issues/1098)
