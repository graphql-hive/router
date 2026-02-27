---
hive-router: patch
hive-router-plan-executor: patch
---

Fix missing elements in the introspection;

- `isDeprecated` and `deprecationReason` fields in introspection results for input values. This caused deprecated input values to be treated as non-deprecated, which could lead to clients not being aware of deprecations and potentially using deprecated fields or arguments.

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

- `isOneOf` field in introspection results for input object types. This field indicates whether an input object type is a "oneOf" type, which is a special kind of input object that allows only one of its fields to be provided. The absence of this field could lead to clients not being able to correctly identify and handle "oneOf" input object types.

```graphql
{
  __type(name: "SomeInputObjectType") {
    name
    kind
    isOneOf # This field was missing, causing clients to not be able to identify "oneOf" input object types
  }
}
```

- `defaultValue` field in introspection results for input values and arguments. This field provides the default value for an argument if it is not provided in a query. The absence of this field could lead to clients not being aware of default values for arguments, which could result in unexpected behavior when executing queries that rely on default argument values.

```graphql
{
  __type(name: "SomeType") {
    fields {
      name
      args {
        name
        defaultValue # This field was missing, causing clients to not be aware of default argument values
      }
    }
  }
}
```