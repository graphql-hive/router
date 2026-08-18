---
hive-router: patch
---

Fix `@requiresScopes`/`@authenticated` on union members being combined into a single
all-or-nothing rule for the whole union, instead of being checked independently per member.

Given:

```graphql
union Media = Book | Movie

type Book @requiresScopes(scopes: [["a", "b"]]) {
  title: String
}

type Movie @requiresScopes(scopes: [["c", "d"]]) {
  title: String
}

type Query {
  media: Media!
}
```

Before this fix, we required the following scopes for each field in the given query:

```graphql
{
  media { # a + b + c +d 
    __typename # a + b + c +d ((inherited from media's gate))
    ... on Book { title } # a + b (but it wasn't reachable because "media" validation gated it with a+b+c+d)
    ... on Movie { title } # c + d (but it wasn't reachable because "media" validation gated it with a+b+c+d)
  }
}
```

And with the fix, now: 

```graphql
{
  media { # none
    __typename # none
    ... on Book { title } # a + b
    ... on Movie { title } # c + d
  }
}
```
