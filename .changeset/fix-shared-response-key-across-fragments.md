---
hive-router-plan-executor: patch
hive-router: patch
---

Fix authorized fields being nulled out when a sibling fragment rejects a field of the same response key.

Given:

```graphql
union Media = Book | Movie

type Book @requiresScopes(scopes: [["a", "b"]]) {
  title: String
}

type Movie @requiresScopes(scopes: [["c", "d"]]) {
  title: String
}
```

A token granted only `a` + `b` (fully satisfying `Book`, not `Movie`) querying:

**Before:** rejected fields were tracked only by their flattened response-key path
(`media.title`), with no notion of which fragment they came from. `Book.title` and
`Movie.title` collapsed into the same entry, so rejecting `Movie`'s `title` nulled
out `Book`'s too, even though it was correctly authorized on its own.

```graphql
{
  media {
    ... on Book { title }    # a + b - granted
    ... on Movie { title }   # c + d - not granted
  }
}
```

**After:** inline fragments are also tracked as type conditions, so sibling
fragments selecting the same field name are tracked independently:

```graphql
{
  media {
    ... on Book { title }    # kept
    ... on Movie { title }   # rejected
  }
}
```
