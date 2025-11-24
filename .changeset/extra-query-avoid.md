---
query-planner: patch
router: patch
node-addon: patch
---

# Avoid extra `query` prefix for anonymous queries

When there is no variable definitions and no operation name, GraphQL queries can be sent without the `query` prefix. For example, instead of sending:

```diff
- query {
+ {
  user(id: "1") {
    name
  }
}
```
