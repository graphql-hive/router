---
query-planner: patch
graphql-tools: patch
executor: patch
node-addon: patch
router: patch
---

# Moves `graphql-tools` to router repository

This change moves the `graphql-tools` package to the Hive Router repository.

# Own GraphQL Parser

This change also introduces our own GraphQL parser (copy of `graphql_parser`), which is now used across all packages in the Hive Router monorepo. This allows us to have better control over parsing and potentially optimize it for our specific use cases.
