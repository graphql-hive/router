---
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-router: patch
node-addon: patch
---

# Added missing `isRepeatable` on `type __Directive`

The router's introspection schema was resolving `isRepeatable`, but it did not appear in the public (consumer) schema, leading to validation errors when introspection schema was executed through Laboratory. 

This change adds the missing `isRepeatable: Boolean!` to `type __Directive`, according to the [GraphQL introspection spec](https://github.com/graphql/graphql-spec/blob/main/spec/Section%204%20--%20Introspection.md).
