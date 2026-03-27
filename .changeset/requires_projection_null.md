---
hive-router-plan-executor: patch
hive-router: patch
---

# Fix null field handling in entity request projection

Fixed a bug in entity request projection where present `null` fields could be mishandled, which in some nested projection paths could also lead to malformed JSON output. [#880](https://github.com/graphql-hive/router/issues/880).
