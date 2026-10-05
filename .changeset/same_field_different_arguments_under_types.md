---
hive-router: patch
node-addon: patch
---

# Fix a crash when one field has different arguments under different types

Asking for the same field with different arguments under two types crashed the router's worker, and the client got no response.

```graphql
query {
  storefront {
    departments {
      ... on Aquatics {
        photo {
          thumbnail(width: 100)
        }
      }
      ... on Reptiles {
        photo {
          thumbnail(width: 200)
        }
      }
    }
  }
}
```

Before: `panicked at ... Unexpected conflict`.

In the example above, the router combined the two `Photo` lookups into one, and the two `thumbnail` fields can't be combined. After: the two lookups stay separate, and they still go out in one request.

Closes https://github.com/graphql-hive/router/issues/1311
