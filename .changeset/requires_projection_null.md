---
hive-router-plan-executor: patch
hive-router: patch
---

# Fix malformed `_entities` request when `@requires` data is null

Fixed a bug where request projection could produce malformed JSON when a nested field was null [#880](https://github.com/graphql-hive/router/issues/880).
