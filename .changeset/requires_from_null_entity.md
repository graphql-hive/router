---
hive-router: patch
---

# Fix `@requires` fields from an entity resolved to `null`

When the subgraph providing a `@requires` field returned `null` for the entity, the router could skip the call that needed that field, and silently drop data.

```graphql
type FavoritesPage @key(fields: "__typename") {
  petCount: Int # from `favorites`
  searchBox: SearchBox! @requires(fields: "petCount") # from `search`
}
```

```graphql
query {
  favoritesPage {
    searchBox {
      placeholder
    }
  }
}
```

Before: `favorites` answered `_entities: [null]`, so the page had no `petCount`. The router left the page out of the call to `search`, `searchBox` was missing, and being non-null, it turned the whole page into `null`, without an error:

```json
{ "data": { "favoritesPage": null } }
```

After: an entity is left out only when none of its keys in that subgraph is complete, as the subgraph can't find it then. A missing `@requires` field is sent as `null`, so `search` gets `{ "__typename": "FavoritesPage", "petCount": null }`:

```json
{ "data": { "favoritesPage": { "searchBox": { "placeholder": "Find a pet" } } } }
```

With a key other than `__typename`, the entity was already sent, without the field. It now has the field, set to `null`.

Closes https://github.com/graphql-hive/router/issues/1669
