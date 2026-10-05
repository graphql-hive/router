---
hive-router: patch
node-addon: patch
---

# Fix a client alias with the same name as a key field

When a query used a key field's name as an alias for another field, like `id: name`, that field's value replaced the key. Later requests that needed the key then sent the wrong value, for example the user's name as their `id`.

```graphql
query ($x: Boolean!) {
  userInA {
    id: name
    aName @include(if: $x) # needs `name`, looked up by the user's `id`
  }
}
```

Before, the call for `aName` read `id` after `id: name` had overwritten it:

```graphql
# a
userInA { __typename id }
# b
... on User { id: name }
# a, for aName
... on User { __typename name id }
```

After, the key is kept under a name of its own:

```graphql
# a
userInA { __typename _internal_qp_alias_0: id }
# a, for aName
... on User { __typename name id: _internal_qp_alias_0 }
```
