---
hive-router: patch
---

# Union members only ask for their own entity fields

- Union members only ask for their own fields of a shared entity type, instead of every member's fields for every entity ([#1732](https://github.com/graphql-hive/router/issues/1732)).
- Entity calls for the same type at other paths stay apart when they ask for a field under another alias or with other arguments. Merged, every entity got both, and an `@fromContext` argument read for one type came out `null` for the other, which could overwrite the right value.
