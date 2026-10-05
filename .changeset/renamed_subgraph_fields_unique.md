---
hive-router: patch
node-addon: patch
---

# Fix invalid subgraph requests from temporary field names

Sometimes the router sends a field to a subgraph under a temporary name, like `_internal_qp_alias_0`, and restores the real name in the response.

For example, when the subgraph declares the field with a different type on two object types. Two problems could make the subgraph reject the request:

- The router compared only the outer type, so `[Item]` and `Item` could end up under one name.
- It checked only the fields right next to the renamed one. So it could reuse a temporary name that was already taken elsewhere in the same object, by another renamed field or by the client.

```graphql
query {
  i {
    _internal_qp_alias_0: __typename
    ... on TypeA {
      strField
    }
    ... on TypeB {
      strField
    }
  }
}
```

Before, two different fields went out under one name:

```graphql
_internal_qp_alias_0: __typename
... on TypeB { _internal_qp_alias_0: strField }
```

After:

```graphql
_internal_qp_alias_0: __typename
... on TypeB { _internal_qp_alias_1: strField }
```

If the planner would still put two different fields under one name, planning now fails with an error instead of sending an invalid request.
