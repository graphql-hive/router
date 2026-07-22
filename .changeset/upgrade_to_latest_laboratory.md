---
hive-router: patch
---

# Upgrade to latest laboratory

[@graphql-hive/laboratory@0.2.0](https://github.com/graphql-hive/console/releases/tag/%40graphql-hive%2Flaboratory%400.2.0)

Remove the request retry setting from the laboratory.

Retries are the wrong primitive for an interactive GraphQL IDE (the
user re-runs operations, and schema introspection already polls), and the underlying HTTP executor
retried on any GraphQL errors response while dropping request headers on the retry, so retries
went out unauthenticated. Existing persisted retry values are ignored automatically.
