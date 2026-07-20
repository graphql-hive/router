---
hive-router-plan-executor: patch
hive-router-query-planner: patch
hive-router: patch
graphql-tools: patch
node-addon: patch
hive-console-sdk: patch
hive-router-internal: patch
hive-apollo-router-plugin: patch
---

# Support custom GraphQL root type names

Hive Router now reads `query`, `mutation`, and `subscription` root type names from the schema instead of assuming they are named `Query`, `Mutation`, and `Subscription`.
