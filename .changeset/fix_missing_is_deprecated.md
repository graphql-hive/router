---
hive-router: patch
hive-router-plan-executor: patch
---

Fix missing `isDeprecated` field in introspection results for input values. This caused deprecated input values to be treated as non-deprecated, which could lead to clients not being aware of deprecations and potentially using deprecated fields or arguments.

```graphql
{
  __type(name: "SomeInputType") {
    inputFields {
      name
      isDeprecated # This field was missing, causing deprecated input values to be treated as non-deprecated
    }
  }
}
```