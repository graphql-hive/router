---
hive-router-plan-executor: patch
hive-router: patch
---

# Introspection Bug Fix

Fixed an issue where, when introspection is disabled, querying root `__typename` was incorrectly rejected (https://github.com/graphql-hive/router/issues/871).
