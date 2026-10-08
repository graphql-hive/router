# @graphql-hive/router-query-planner changelog
## 0.0.49 (2026-10-08)

### Fixes

#### Make query plan cache keys more robust

The query plan cache now uses a 128-bit BLAKE3 digest over an injective encoding of the operation. This guarantees that distinct operations always map to distinct cache keys.

## 0.0.48 (2026-10-06)

### Fixes

#### Fix batched calls running when their fragment's condition is false

Two calls to the same subgraph under the same conditional fragment can be batched into one. The batched call lost the fragment's condition, so it ran even when the condition was false.

```graphql
query ($a: Boolean!, $b: Boolean!) {
  users {
    reviews {
      product {
        upc
        ... on Product @include(if: $a) {
          notes # comes from subgraph `products`
          price @include(if: $b) # comes from subgraph `products`
        }
      }
    }
  }
}
```

Before, the call to `products` ran every time:

```
Flatten(path: "users.@.reviews.@.product|[Product]") {
  Fetch(service: "products") { ... on Product { ... on Product @include(if: $b) { price } ... on Product @include(if: $a) { notes } } }
}
```

After, it only runs when `$a` is true (condition is kept):

```
Include(if: $a) {
  Flatten(path: "users.@.reviews.@.product|[Product]") {
    Fetch(service: "products") { ... on Product { ... on Product @include(if: $b) { price } ... on Product @include(if: $a) { notes } } }
  }
}
```

`reviews` only sends the `__typename` that `products` needs when `$a` is true. With `$a: false`, the call went out without `__typename`, so `products` rejected the whole request, and every other field in that request came back `null`.

#### Fix a client alias with the same name as a key field

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
## a
userInA { __typename id }
## b
... on User { id: name }
## a, for aName
... on User { __typename name id }
```

After, the key is kept under a name of its own:

```graphql
## a
userInA { __typename _internal_qp_alias_0: id }
## a, for aName
... on User { __typename name id: _internal_qp_alias_0 }
```

#### Fix `@requires` fields under a conditional fragment on `Query`

A query with two or more `@requires` fields under a conditional fragment on the root type failed to plan.

```graphql
query ($x: Boolean!) {
  ... on Query @include(if: $x) {
    userInB {
      aName
      cName
    }
  }
}
```

Before: `MissingPathInSelection("|[Query] @include(if: $x).userInB", "Query")`. Moving the first `@requires` call into the root request removed the `... on Query @include(if: $x)` fragment that the second one needed.

After: the query plans correctly and does not error.

#### Fewer subgraph requests for fields under `@include` and `@skip`

A call to another subgraph planned under `@include` or `@skip` always went out as a request of its own, so it could be skipped as a whole. Now it can go into a request to the same subgraph that is sent anyway: one for the same objects, or the one it gets its objects from. Its fields go under `... on T @include(if: $x)`, and the subgraph skips them when the condition is false. No variable value leads to more requests than before.

For example, in this operation:

```graphql
query ($includeRank: Boolean!) {
  cage {
    listings {
      rank @include(if: $includeRank)
    }
  }
}
```

Before, `search` was called twice: once for the listings, and once more, under `$includeRank`, for each listing's `pet`.

After, `pet` is part of the first call, and the condition goes inside the operation:

```graphql
... on Cage {
  listings {
    __typename
    id
    ... on Listing @include(if: $includeRank) { pet { __typename id } }
  }
}
```

#### Fix fields with different arguments, aliases or conditions counted as the same

To decide whether a request already has a field, for example a key or a field a `@requires` needs, the router compared only field names. So it treated these as the same:

| Request has               | Counted as also having, before | After |
| ------------------------- | ------------------------------ | ----- |
| `price(currency: "GBP")`  | `price(currency: "EUR")`       | no    |
| `price(currency: "GBP")`  | `gbp: price(currency: "GBP")`  | no    |
| `a @include(if: $x)`      | `a @include(if: $y)`, or `a`   | no    |
| `... on Cat { whiskers }` | `... on Dog { whiskers }`      | no    |

A request that was still needed could then be dropped as a duplicate, or another request could count on a field it didn't have.

The comparison now fully checks the alias, the arguments, the conditions and the fragments.

#### Fix `@include` dropped from a field after a conditional call to another subgraph

When a field under `@include(if: $x)` needed another subgraph, a later field with the same condition in the same request lost its `@include`.

The subgraph then resolved it even when `$x` was false. The same happened with a conditional fragment like `... on Query @include(if: $x)`.

```graphql
query ($withDetails: Boolean!, $term: String!) {
  record(id: "1") @include(if: $withDetails) {
    ... on User {
      id
      email
      invoices
    } # `invoices` comes from `billing`
  }
  listed: catalog {
    search(term: $term, kind: "item") @include(if: $withDetails) {
      ... on Item {
        id
        label
      }
    }
  }
}
```

Before, `accounts` got `search` without its condition:

```graphql
listed: catalog { search(kind: "item", term: $term) { ... } }
```

After:

```graphql
listed: catalog { search(kind: "item", term: $term) @include(if: $withDetails) { ... } }
```

Closes https://github.com/graphql-hive/router/issues/1647

#### Ensure mutation run in sequence across subgraphs

The fields of a mutation have to run one after another. The router started the next field as soon as the previous field's own request was done, without waiting for the other requests that field needed, like an entity call to another subgraph.

```graphql
mutation {
  create: addProduct(input: { name: "new", price: 599.99 }) {
    # c
    isExpensive # comes from another subgraph
  }
  count: add(num: 1) #c
}
```

Before, `count` ran at the same time as the `isExpensive` call for `create`, because they originated from different subgraphs:

```
Sequence {
  Fetch(service: "a") { create: addProduct(...) }
  Parallel {
    Fetch(service: "c") { count: add(num: 1) }
    Flatten(path: "create") { Fetch(service: "b") { isExpensive } }
  }
}
```

After, `count` waits for it:

```
Sequence {
  Fetch(service: "a") { create: addProduct(...) }
  Flatten(path: "create") { Fetch(service: "b") { isExpensive } }
  Fetch(service: "c") { count: add(num: 1) }
}
```

Consecutive fields that go to the same subgraph are still sent together in one request, in order, if applicable.

#### Fix calls under nested conditions running when an outer condition is false

A call to another subgraph, planned under several conditional fragments or fields, only checked the innermost condition. With an outer condition false and the inner one true, the router could still call the subgraph, for fields the client had skipped.

```graphql
query ($a: Boolean!, $b: Boolean!) {
  userInA {
    id
    ... on User @include(if: $a) {
      ... on User @include(if: $b) {
        name # comes from subgraph `b`
      }
    }
  }
}
```

Before, the call to `b` only checked `$b`:

```
Include(if: $b) {
  Flatten(path: "userInA|[User]|[User]") { Fetch(service: "b") { ... on User { name } } }
}
```

After, it checks both:

```
Include(if: $a) {
  Include(if: $b) {
    Flatten(path: "userInA|[User]|[User]") { Fetch(service: "b") { ... on User { name } } }
  }
}
```

#### Fix `null` for a fragment inside a conditional fragment on an interface

A fragment on an object type inside a conditional fragment on an interface, like `... on Node @include(if: $y) { ... on Cat { eur } }`, lost its own type when the router put the response together. So a field resolved through an `@interfaceObject` came back as `null`.

```graphql
query ($y: Boolean!) {
  things {
    ... on Node @include(if: $y) {
      ... on Cat { eur }
    }
  }
}
```

With `$y: true`, before:

```json
{ "data": { "things": [{ "eur": null }, {}] } }
```

After:

```json
{ "data": { "things": [{ "eur": 1100 }, {}] } }
```

#### Fix `@requires` nested inside another `@requires`

A `@requires` field inside the result of another `@requires` field failed to plan when the outer one needed fields from two different subgraphs.

With `checkup @requires(fields: "weight age")`, where `weight` and `age` come from two subgraphs, and `grade @requires(fields: "fee")` on `Checkup`:

```graphql
query {
  pet {
    checkup {
      grade
    }
  }
}
```

Before: `NonSingleParent(2)` (internal) error, so the router answered with `QUERY_PLAN_BUILD_FAILED`. The planner expected the request for `checkup` to wait for one other request, but it waits for two.

After: the query plans correctly.

Closes https://github.com/graphql-hive/router/issues/1310

#### Fix `MissingStep` when several fields need the same `@requires` chain

Queries asking for several fields that rely on the same chain of `@requires` fields, directly, through fragments or under aliases, could fail with `QUERY_PLAN_BUILD_FAILED`. The planner combines requests in several rounds, and a later round could still point at a request that an earlier round had already combined into another one.

```graphql
query {
  product {
    requestedA: canAffordWithAndWithoutDiscount
  }
  ... on Query {
    product {
      canAffordWithAndWithoutDiscount
    }
  }
  ... on Query {
    product {
      ... {
        ... on Product {
          requestedB: canAffordWithAndWithoutDiscount
        }
      }
      canAfford
    }
  }
}
```

Before: `failed to build fetch graph: MissingStep(...)`. After: the query plans correctly.

#### Fix invalid subgraph requests from temporary field names

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

#### Fix `@requires` that reads fields of an interface's types

A `@requires` reading fields of specific types behind an interface, like `pet { __typename ... on Dog { tricks } ... on Cat { whiskers } }` where `pet` is an `Animal`, failed with `QUERY_PLAN_BUILD_FAILED`.

The router looked up the cats and the dogs separately, then combined the two lookups and treated the result as covering every pet. But a pet can also be a `Bird`. So the combined lookup was then merged with the lookup for every `Animal`, which put `whiskers` and `tricks` on `Animal`, where they don't exist.

```graphql
query {
  listings {
    rank
  }
}
```

Before: `No field found for name 'whiskers' in type 'Animal'`, or `UnexpectedMissingDefinition("Animal")` with `@include` on `rank`. After: the query plans, and `catalog` gets each `pet` once.

Closes https://github.com/graphql-hive/router/issues/1308
Closes https://github.com/graphql-hive/router/issues/1309

#### Fix `@requires` through a union losing its fields

When a `@requires` selected fields through a union, the router kept the union members of the other subgraphs instead of the current one. The required fields were dropped, and the subgraph got an invalid request.

For `price @requires(fields: "book { ... on Media { ... on Book { title } } }")` and this query:

```graphql
query {
  products {
    price
  }
}
```

Before, `catalog` was asked for `book` without any fields:

```graphql
{ products { __typename id book } }
```

After:

```graphql
{ products { __typename id book { title } } }
```

#### Fix wrong values for `@requires` fields with arguments

When `@requires` fields needed the same field with different arguments, like `price(currency: "GBP")` for `gbp` and `price(currency: "EUR")` for `eur`, the router fetched one of them under a temporary name.

It picked these names one request at a time, so a field could be fetched under one name and read under another. The subgraph then computed its field from the wrong value.

In this example:

```graphql
query {
  things {
    ... on Cat {
      gbp
    }
    eur
  }
}
```

Before, `pricing` computed `eur` for cats from `price`, which held the GBP price:

```graphql
## shop
... on Cat { price(currency: "GBP") _internal_qp_alias_0: price(currency: "EUR") }
## what pricing read
... on Cat { price }
```

After, it reads the EUR price:

```graphql
## what pricing reads
... on Cat { price: _internal_qp_alias_0 }
```

The router now picks these names once for the whole query, for each place in the response.

#### Fix a crash when one field has different arguments under different types

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

## 0.0.47 (2026-09-28)

### Fixes

#### Avoid duplicate built-in directive definitions in introspection

Supergraphs that already define a built-in directive, like `@oneOf` (emitted by composition whenever a subgraph uses it), no longer end up with that directive defined twice in the consumer schema.

#### Allow `@deprecated` on arguments in introspection results

The router's built-in `@deprecated` definition now includes the `ARGUMENT_DEFINITION` location, as the GraphQL spec requires.

#### Fix type conditions on a list of an `@interfaceObject`

Queries with a type condition on a list of an `@interfaceObject`, like `friends { ... on User { name } }`, no longer fail with `No paths found for selection item`.

#### Faster Processing of Large Subgraph Responses

The Router no longer sorts the fields of every object in a subgraph response. Objects now keep the order the subgraph sent them in, and the Router finds each field by checking the next one first, since responses follow the order of the query.

This removes a sort per object and a search per field. In our test with a single subgraph returning a 2.8MB response, the time the Router spends on each request dropped by about 10%, and throughput increased by about 5%.

#### Validate list literals in more positions

Validation now checks the items of a list literal whose type is a non-null list. For example, `query ($v: [Color]! = [PURPLE])` is rejected when `PURPLE` is not a `Color` value. Before, those items were not checked, so an invalid default was only caught when it was used, or was sent to the subgraph unchecked.

A list literal in a position that doesn't expect a list, like `query ($v: Color = [RED])` or `query ($v: Int = [1])`, is now rejected with a `ValuesOfCorrectType` error. Custom scalars still accept any literal, as described by the GraphQL specification.

#### Keep `@provides` fields on the path that provides them

Fields made available by `@provides` could leak to another field returning the same type, like `Store.orders` getting the fields `User.orders` provides. The planner then skipped a fetch it needed. Provided fields now only count on the path of the field that provides them.

#### Fix extra fetches for `@requires` after an entity hop

- `@requires` after an entity hop no longer adds an extra fetch to the same subgraph ([#1539](https://github.com/graphql-hive/router/issues/1539)).
- Aliased `@requires` fields with different arguments (like `price(currency: USD)` and `price(currency: EUR)`) now get the right values after an entity hop.

Closes https://github.com/graphql-hive/router/issues/1539

#### Use `@provides` fields for `@requires`

When the fields a `@requires` needs are made available by a `@provides` on the way, the field is now fetched together with them, from the same subgraph. Previously the planner ignored the provided fields and made extra `_entities` calls to get them again.

#### Fix `@requires` on an object of the same type nested in an entity call

A `@requires` field on an object nested inside an entity of the same type, like `user { other { label } }` where both `user` and `other` are a `User`, no longer fails with `MissingPathInSelection`.

#### Fix progressive `@override` on fields returning a union

When a field returning a union was progressively overridden (`@override(from: ..., label: ...)`), the overriding subgraph was still used when the label was off. The label is now respected, like it is for other fields.

#### Variable Coercion Errors Follow the GraphQL Spec

Errors for invalid variable values now use the graphql-js wording, name the variable, and point to the invalid list item. For example, `$input: [String!]` given `[0, 1]` now returns:

```
Variable "$input" has invalid value at [0]: String cannot represent a non string value: 0
```

Values in these messages are printed the way graphql-js prints them (for example `[1]`), not as internal router values. Errors caused by a variable's default value say `has invalid default value`.

#### Variable definitions can now have a description

As allowed by the September 2025 GraphQL spec (`query ("The user ID" $id: ID!) { ... }`)

## 0.0.46 (2026-09-21)

### Fixes

#### Use Less Memory for Response Projection Plans

The Router now stores response projection plans in a more compact form.

This reduces the memory used when the Router handles many different queries. In our memory test, each new query used about 20% less memory.

## 0.0.45 (2026-09-15)

### Fixes

#### Reduce memory retained by cached query and projection plans

Planner data is built using growable collections and then stored in caches, which can leave unused vector capacity allocated. The router now recursively shrinks data structures before caching them.

In tested queries, it resulted in a 50% reduction in retained heap memory.

## 0.0.44 (2026-09-02)

### Features

- Expose `computeCacheKey` fn

## 0.0.43 (2026-07-26)

### Fixes

#### Support custom GraphQL root type names

Hive Router now reads `query`, `mutation`, and `subscription` root type names from the schema instead of assuming they are named `Query`, `Mutation`, and `Subscription`.

## 0.0.42 (2026-07-20)

### Fixes

#### Improve GraphQL operation validation

- **Faster validation (2-3x):** rules now share a single `OperationVisitor` pass over the operation document instead of each rule visiting it independently.
- **New `UniqueInputFieldNames` rule:** input object fields are now kept as a list rather than a map, so duplicate fields are no longer silently deduplicated before validation. A query like `{ field(input: { value: 1, value: 2 }) }` is now correctly rejected.
- **Fixed `VariablesInAllowedPosition`:** now accounts for default values on variables, field arguments, and input object fields. Nullable variables used in a non-null argument that defines a default are no longer incorrectly rejected.

## 0.0.41 (2026-07-07)

### Fixes

#### Query Planning performance improvements

Removed unused per-path edge tracking and switched to references instead of owned values - no cloning of selection items.

## 0.0.40 (2026-07-06)

### Fixes

#### Fix stack overflow on cyclic fragment spreads with sibling fields or directives

A self-referential fragment that also selects a sibling field (`fragment A on Query { x ...A }`) or puts a directive on the cycling spread (`...A @include(if: $c)`) caused unbounded recursion during fragment inlining in normalization, overflowing the stack and crashing the process.

#### Fixed false circular dependency detection in case of `@requires`

We fixed a query planner bug that could make some valid federated queries fail.

The issue happened when planning fields with nested `@requires` data. The planner compared required selection sets using only the top-level field, ignoring the rest of the selection set. For example, `foo { bar }` and `foo { baz { qux } }` could both be treated as overlapping `foo`.

This could make the planner drop a valid way to fetch the required data too early.

## 0.0.39 (2026-07-02)

### Fixes

#### Restrict indirect path finding to valid subgraphs

Fix an issue where query planning could appear stuck on some complex federated schemas due to excessive indirect path exploration.

Indirect path exploration happens when the planner cannot resolve a field directly in the current subgraph and starts searching for a route through other subgraphs that could satisfy the field and its requirements.

Indirect field lookup is now limited to subgraphs that can actually resolve the requested field, reducing unnecessary work and preventing planning stalls.

## 0.0.38 (2026-07-01)

### Fixes

#### Better detection of mutations in query-planner

When a mutation is encountered in an operation (e.g. `mutation { ... }`), the query planner needs to use `Sequence` instead of `Parallel` to ensure the mutation is executed in the correct order.

Previuosly, Hive Router was checking if `type Mutation` was used in the root step to determine if a mutation was present.

This change uses the actual incoming operation type (`mutation { ... }`) to determine if a mutation is present in a specific plan.

#### Fix root type re-entry

When a field re-exposes a root type from a nested position (e.g. a mutation field
returning a type with a `query: Query` field), the query planner could not resolve
selections that live in a different subgraph, and the executor merged whatever it
did fetch at the response root instead of the nested path — so those fields
resolved to `null`, or failed to resolve fully. 

Fixes [#1164](https://github.com/graphql-hive/router/issues/1164)

## 0.0.37 (2026-06-30)

### Fixes

- Upgrade `ntex` to latest and pin versions

## 0.0.36 (2026-06-30)

### Fixes

#### Prevent @requires creating a circular dependency across subgraphs

The Query Planner could hit a timeout when a field with `@requires` needed to move to another subgraph, and the move required the same fields.

#### Fix: Traverse fragments linearly during cycle validation and inlining

GraphQL fragments can spread other fragments, e.g. `fragment A on T { ...B }`. When fragments form a long acyclic chain (A spreads B, B spreads C, and so on for thousands of links), we walked that chain with plain recursion. 

This change prevents the stack from being filled in such cases.

## 0.0.35 (2026-06-24)

### Fixes

#### Fix missing `__typename` with `@requires` re-entry

Resolving a field with `@requires` makes the planner re-enter the entity's subgraph through `_entities`, using a representation built from the entity's data. That representation must carry the entity's `__typename`, since `_entities` routes on it.

The fetch that produces the entity now always selects its `__typename`, so the re-entry representation is complete.

Fixes [#1070](https://github.com/graphql-hive/router/issues/1070)

## 0.0.34 (2026-06-18)

### Fixes

#### Improve handling of unions

The query planner improves handling of union types whose members vary between subgraphs. Previously, the planner always computed an intersection of union members, ignoring subgraph-specific members.

Fixes [#1098](https://github.com/graphql-hive/router/issues/1098)

## 0.0.33 (2026-06-17)

### Fixes

#### Add an experimental query planner option, `experimental_abstract_type_folding`

```yaml
query_planner:
    experimental_abstract_type_folding: true # false by default
```

Folds matching concrete object-type fragments in subgraph calls, into a shared interface fragment even when that interface is not the field's declared return type.

It's an opt-in addition to [`011be5b`](https://github.com/graphql-hive/router/commit/011be5bdbfb00bf1e415eb7a50e6be91f565ef05).

```diff
## queries `product-service` subgraph
query {
  products {
-    ... on Book  { id title }
-    ... on Movie { id title }
+    ... on Media { id title }
  }
}
```

The `products` field returns `Product` interface, but one object-type member of this interface called `Album` is not present in the query, therefore `... on Product {...}` is not possible to use (default behavior). With the feature flag enabled, both fragments are folded into `... on Media { ... }`, because `Book` and `Movie` are the only members of the `Media` interface in the `product-service` subgraph.

#### Avoid indirect lookup for directly resolved leaf fields

The planner now skips indirect path lookup when a leaf field already has a valid direct path.

## 0.0.32 (2026-06-16)

### Fixes

#### Fix union list FieldMove creation

In some cases union list was treated as single union field in graph.

## 0.0.31 (2026-06-15)

### Fixes

#### Demand Control with `@cost` and `@listSize` directives

Add support for the [Demand Control specification](https://ibm.github.io/graphql-specs/cost-spec.html), allowing operators to limit the cost of incoming GraphQL operations using the `@cost` and `@listSize` directives.

The router now calculates the cost of incoming operations based on directive-driven type, field, and argument costs (with list-size estimation) and can reject operations that exceed a configured maximum. Both static (request) and actual (response) cost can be measured, and the behavior is configurable via the new `demand_control` section in the router configuration.

Telemetry is included: new metrics under `demand_control_metrics` and additional span attributes expose estimated/actual cost and rejection reasons for observability.

[Documentation for the feature is available here](https://the-guild.dev/graphql/hive/docs/router/security/demand-control)

## 0.0.30 (2026-06-13)

### Fixes

#### Fold repeated object-type selections into a single interface selection

When a `Fetch` node asks for the same fields on different object types, and all
of those types implement the same interface that matches the field's return type,
the query planner now merges them into a single inline fragment on the interface
instead of keeping separate branches.

For example: `query { media { ... on Book { id title } ... on Movie { id title } } }` becomes
`query { media { id title } }` when the field's return type is `Media` and both
`Book` and `Movie` implement it in the subgraph.

## 0.0.29 (2026-06-03)

### Fixes

#### Forward operation name to subgraphs

Added the `traffic_shaping.all.forward_operation_name` and `traffic_shaping.subgraphs.<name>.forward_operation_name` options. The option defaults to `false`.

The operation name is injected (opt-in) into the query document and the `operationName` JSON field, formatted as `<client_operation_name>__<fetch_step_id>`, when sending requests to subgraphs.

Global opt-in:

```yaml
traffic_shaping:
  all:
    forward_operation_name: true
```

Per-subgraph opt-in:

```yaml
traffic_shaping:
  subgraphs:
    products:
      # Overrides global setting for this subgraph
      forward_operation_name: true
```

## 0.0.28 (2026-05-27)

### Fixes

#### Fix `VariablesInAllowedPosition` rejecting list-typed variables with a non-null default value

The router used to reject valid client queries that declared a list-typed variable with a non-null default value, for example:

```graphql
query Q($arg: [SomeEnum!] = SOME_VALUE) {
  field(arg: $arg)
}
```

with a `VariablesInAllowedPosition` validation error containing a malformed type:

```
Variable "$arg" of type "SomeEnum!!" used in position expecting type "[SomeEnum!]".
```

The rule used to compute the variable's effective type incorrectly when the variable was list-typed and had a non-null default value: it dropped the list wrapper and re-wrapped the inner element type in `NonNull`, producing the invalid `T!!` shape. Per [the spec](https://spec.graphql.org/draft/#sec-All-Variable-Usages-are-Allowed), a non-null default value makes the variable usable in a non-null position; the variable's effective type should be `NonNull(var_type)`, not `NonNull(element_type)`. So for `[SomeEnum!]` with a non-null default, the effective type is now correctly `[SomeEnum!]!` (and the query is accepted).

## 0.0.27 (2026-05-11)

### Fixes

#### Escape inline string arguments when emitting subgraph operations

Fixes a bug where string values inlined as arguments in subgraph operations were not re-escaped per the GraphQL spec. When an incoming operation contained a string literal whose decoded value carried a quote or backslash (for example `payload: "\"quoted\""`), the router forwarded the argument to the subgraph as `payload: ""quoted""`, producing invalid GraphQL. The same went for newlines, tabs, and other control characters.

Now the characters are escaped properly per the [GraphQL spec](https://spec.graphql.org/draft/#StringCharacter).

## 0.0.26 (2026-05-11)

### Fixes

#### Preserve custom scalars as raw JSON

Custom scalar fields marked by the query planner are now preserved as raw JSON instead of being parsed and rebuilt as structured response values. This improves correctness for JSON passthrough custom scalars while avoiding performance regressions for normal response handling.

## 0.0.25 (2026-05-08)

### Features

#### Fix conditional directive handling in response projection.

This fixes several edge cases where `@skip` and `@include` could produce an incorrect final response after query planning and projection planning.

## 0.0.24 (2026-05-05)

### Fixes

- Adjustments in operation's kind being Enum and not &'static str

#### Added missing `isRepeatable` on `type __Directive`

The router's introspection schema was resolving `isRepeatable`, but it did not appear in the public (consumer) schema, leading to validation errors when introspection schema was executed through Laboratory. 

This change adds the missing `isRepeatable: Boolean!` to `type __Directive`, according to the [GraphQL introspection spec](https://github.com/graphql/graphql-spec/blob/main/spec/Section%204%20--%20Introspection.md).

#### Avoid propagating `@include`/`@skip` conditions to unconditional fetches

Fixed query planner condition propagation logic to avoid wrapping unconditional fetches
in conditional blocks when merging steps. This ensures that fields without directives are
not incorrectly gated by conditions from other steps, allowing for correct execution of
queries with mixed conditional and unconditional selections.

#### Fix fragments being dropped when multiple inline fragments target the same concrete type within an abstract type fragment.

Previously, when a query contained two or more inline fragments on the same concrete type nested inside an interface or union fragment, only the first fragment's fields were included in the query plan — all subsequent ones were silently dropped.

**Example query that previously returned only `title`:**

```graphql
query {
  films {
    ... on Node {
      ... on Film { title }
      ... on Film { director }
    }
  }
}
```

Both fields are now correctly returned.

#### Fix fragment handling

Fix fragment handling for some queries that use reusable fragments with conditional directives

## 0.0.23 (2026-04-20)

### Fixes

#### Fix query planner handling for combined `@skip` and `@include` conditions.

- Preserve both directives when converting inline fragment conditions into fetch step selections
- Build the expected nested condition nodes for combined skip/include execution paths
- Handle `SkipAndInclude` in selection matching, fetch-step rendering, and multi-type batch path hashing
- Add regression snapshot tests for field-level and fragment-level combined conditions

For example a query like this:

```graphql
query($skip: Boolean!, $include: Boolean!) {
  user {
    name @skip(if: $skip) @include(if: $include)
  }
}
```

Will now correctly generate a fetch step with an inline fragment that has both `@skip` and `@include` conditions, and the planner will properly evaluate the combined conditions when determining which selections to include in the execution plan.

- `@skip(if: $skip)` is true, the selection will be skipped regardless of the `@include` condition.
- `@include(if: $include)` is false, the selection will be skipped regardless of the `@skip` condition.
- Only if `@skip(if: $skip)` is false and `@include(if: $include)` is true, the selection will be included in the execution plan.

## 0.0.22 (2026-04-15)

### Fixes

- Fix `Subscription.primary` type to `FetchNode` instead of `PlanNode` in the distributed `index.d.ts` file.

## 0.0.21 (2026-04-15)

### Fixes

#### `Subscription` node's `primary` is `FetchNode` instead of `PlanNode` now, but the types were not compatible.

This change updates the type of `Subscription.primary` to be `FetchNode` instead of `PlanNode`.

## 0.0.20 (2026-04-15)

### Features

#### Query Plan Subscriptions Node

The query planner now emits a `Subscription` node when planning a subscription operation. The `Subscription` node contains a `primary` fetch that is sent to the subgraph owning the subscription field.

## 0.0.19 (2026-04-15)

### Features

#### Query Plan Subscriptions Node

The query planner now emits a `Subscription` node when planning a subscription operation. The `Subscription` node contains a `primary` fetch that is sent to the subgraph owning the subscription field.

## 0.0.18 (2026-04-13)

### Fixes

#### Fix planning for conditional inline fragments and field conditions

Fixed a query-planner bug where directive-only inline fragments (using `@include`/`@skip` without an explicit type condition) could fail during normalization/planning for deeply nested operations.

This update improves planner handling for conditional selections and adds regression tests to prevent these failures in the future.

## 0.0.17 (2026-04-01)

### Fixes

- This patch includes the fixes in the query planner including the fixes for mismatch handling so conflicting fields are tracked by response key (alias-aware), and internal alias rewrites restore the original client-facing key (alias-or-name) instead of always the schema field name.

## 0.0.16 (2026-03-16)

### Fixes

- Add missing `*.node` binaries to the `dist` folder in the distributed package.

## 0.0.15 (2026-03-16)

### Features

- progressive override (#856)

#### Introduce BatchFetch for compatible entity fetches to improve query performance

When multiple `Flatten(Fetch)` steps target the same subgraph and have compatible shape, the planner can group them into one batched fetch operation with aliases.

Batching keeps execution depth the same, but **reduces request fanout**.
In our benchmark query, **downstream requests drop from `13` to `7`** while the number of execution waves stays unchanged.
This should also reduce pressure on subgraphs, because entities are resolved in one batched subgraph call instead of being resolved across multiple incoming GraphQL requests, where the lack of DataLoader or another caching layer could otherwise cause duplicate resolution work.

Before: 

```graphql
Parallel {
  Flatten(path: "products.@") {
    Fetch(service: "inventory") {
      {
        ... on Product {
          upc
        }
      } =>
      {
        ... on Product {
          shippingEstimate
        }
      }
    }
  }
  Flatten(path: "topProducts.@") {
    Fetch(service: "inventory") {
      {
        ... on Product {
          upc
        }
      } =>
      {
        ... on Product {
          shippingEstimate
        }
      }
    }
  }
}
```

After:

```graphql
BatchFetch(service: "inventory") {
  {
    _e0 {
      paths: [
        "products.@"
        "topProducts.@"
      ]
      {
        ... on Product {
          upc
        }
      }
    }
  }
  {
    _e0: _entities(representations: $__batch_reps_0) {
      ... on Product {
        shippingEstimate
      }
    }
  }
}
```

When two entity fetches go to the same subgraph but request different output fields, they are batched into one `BatchFetch` node with two aliases, but share the same variables, to reduce the payload size.

```
BatchFetch(service: "inventory") {
  {
    _e0 {
      paths: [
        "products.@"
      ]
      {
        ... on Product {
          upc
        }
      }
    }
    _e1 {
      paths: [
        "products.@"
      ]
      {
        ... on Product {
          upc
        }
      }
    }
  }
  {
    _e0: _entities(representations: $__batch_reps_0) {
      ... on Product {
        shippingEstimate
      }
    }
    _e1: _entities(representations: $__batch_reps_0) {
      ... on Product {
        inStock
      }
    }
  }
}
```

#### Public API Changes

### Progressive Override support in `QueryPlanner.plan`

Now `QueryPlanner.plan` accepts two additional parameters: `activeLabels` and `percentageValue`. These parameters are used to determine which overrides should be applied when generating the query plan. The `activeLabels` parameter is a set of labels that are currently active, and the `percentageValue` parameter is a number between 0 and 100 that represents the percentage of traffic that should be routed to the overrides.

### `AbortSignal` support in `QueryPlanner.plan`

The `QueryPlanner.plan` method now also accepts an optional `signal` parameter of type `AbortSignal`. This allows the caller to abort the query planning process if it takes too long or if the user cancels the operation. If the signal is aborted, the `plan` method will throw an error.

### `overrideLabels` and `overridePercentages` getters

Two new getters have been added to the `QueryPlanner` class: `overrideLabels` and `overridePercentages`. The `overrideLabels` getter returns a set of all the labels that are defined in the planner's supergraph, while the `overridePercentages` getter returns an array of all the percentage values that are defined in the planner's supergraph. These getters can be used by the caller to determine which overrides are available and how they are configured.

### `QueryPlanner.plan` is no longer a `Promise`

The `QueryPlanner.plan` method is now a synchronous method that returns a `QueryPlan` directly, instead of returning a `Promise`. This change was made to simplify the API and to allow for better error handling. If the query planning process encounters an error, it will throw an exception that can be caught by the caller.

### `QueryPlanner.planAsync` is now a `Promise`

The `QueryPlanner.planAsync` method is now an asynchronous method that returns a `Promise` that resolves to a `QueryPlan`. This method is intended for use cases where the query planning process may take a long time, and the caller wants to avoid blocking the main thread. The `planAsync` method accepts the same parameters as the `plan` method, including the new `activeLabels`, `percentageValue`, and `signal` parameters.

### `QueryPlanner` constructor now uses `safe_parse_schema`

The `QueryPlanner` constructor now uses the `safe_parse_schema` function to parse the supergraph SDL. This function is a safer alternative to the previous parsing method, as it returns a `Result` that can be handled gracefully in case of parsing errors. If the SDL cannot be parsed, the constructor will return an error instead of panicking.

## Implementation changes

- The `QueryPlanner` struct now holds a `Planner` instance directly, instead of an `Arc<Planner>`. This change was made to simplify the internal implementation and to avoid unnecessary reference counting. Since the `QueryPlanner` is not designed to be shared across threads, there is no need for the additional overhead of an `Arc`.

- `AbortSignal` and `CancellationToken` integration to give the ability to cancel the query planning process to the Node addon consumer.

- `QueryPlanner.planAsync` is introduced with [`AsyncTask`](https://napi.rs/docs/concepts/async-tasks) to allow for non-blocking query planning in the Node addon.

## 0.0.14 (2026-03-12)

### Features

#### Metrics with OpenTelemetry and Prometheus

This release adds support for OpenTelemetry metrics. In addition to existing tracing support, the router can now collect detailed metrics about HTTP and GraphQL activity and export them to a Prometheus endpoint or to an OTLP collector.

- Telemetry configuration now has a `metrics` section. Users can enable metrics exporters and tune histogram buckets under `telemetry.metrics` in `router.config.yaml`. By default metrics are disabled, so existing configurations continue to work unchanged.
- **Prometheus exporter** exposes a `/metrics` endpoint that follows the standard Prometheus text format. It can be attached to Router's http server or run on its own port. 
- **OTLP exporter** is available for sending metrics to an OpenTelemetry collector via gRPC or HTTP.
- **Instrumentation for every stage of the pipeline** - parsing, normalization, validation, planning and execution.
- **HTTP client/server metrics** - Router records metrics for incoming HTTP requests (latencies, sizes and status codes) and for outbound subgraph requests. These instruments follow the OpenTelemetry HTTP semantic conventions, making them usable out‑of‑the‑box with observability backends.
- **Supergraph reload metrics** - polling and reloading the supergraph is measured with poll counts, durations and errors, giving visibility into slow or failed schema reloads.

**Example configuration**

```yaml
telemetry:
  metrics:
    exporters:
      - prometheus:
          enabled: true
          # optional custom path (default `/metrics`)
          path: /metrics
          # serve on this port
          port: 9090
      - otlp:
          enabled: true
          # An absolute path to the OpenTelemetry collector
          endpoint: "http://otel-collector:4317"
          # protocol can be `grpc` or `http`
          protocol: http
    instrumentation:
      instruments:
        # Disable HTTP server request duration metric
        http.server.request.duration: false
        http.client.request.duration:
          attributes:
            # Disable the label
            graphql.operation.name: false
```

Visit ["OpenTelemetry Metrics" documentation](https://the-guild.dev/graphql/hive/docs/router/observability/metrics) for more details on configuring metrics and exporters.

## 0.0.13 (2026-03-05)

### Features

#### Improve Query Plans for abstract types

The query planner now combines fetches for multiple matching types into a single fetch step.
Before, the planner could create one fetch per type.
Now, it can fetch many types together when possible, which reduces duplicate fetches and makes query plans more efficient.

#### Rename internal query-plan path segment from `Cast(String)` to `TypeCondition(Vec<String>)`

Query Plan shape changed from `Cast(String)` to `TypeCondition(Vec<String>)`.
The `TypeCondition` name better reflects GraphQL semantics (`... on Type`) and avoids string encoding/decoding like `"A|B"` in planner/executor code.

**What changed**
- Query planner path model now uses `TypeCondition` terminology instead of `Cast`.
- Type conditions are represented as a list of type names, not a pipe-delimited string.
- Node addon query-plan typings were updated accordingly:
  - `FetchNodePathSegment.TypenameEquals` now uses `string[]`
  - `FlattenNodePathSegment` now uses `TypeCondition: string[]` (instead of `Cast: string`)

## 0.0.12 (2026-02-06)

### Features

- Operation Complexity - Limit Aliases (#746)
- Operation Complexity - Limit Aliases (#749)

## 0.0.11 (2026-01-22)

### Fixes

#### Refactor Parse Error Handling in `graphql-tools`

Breaking;
- `ParseError(String)` is now `ParseError(InternalError<'static>)`.
- - So that the internals of the error can be better structured and more informative, such as including line and column information.
- `ParseError`s are no longer prefixed with "query parse error: " in their Display implementation.

## 0.0.10 (2026-01-14)

### Fixes

#### Moves `graphql-tools` to router repository

This change moves the `graphql-tools` package to the Hive Router repository.

## Own GraphQL Parser

This change also introduces our own GraphQL parser (copy of `graphql_parser`), which is now used across all packages in the Hive Router monorepo. This allows us to have better control over parsing and potentially optimize it for our specific use cases.

## 0.0.9 (2025-12-11)

### Fixes

- chore: Enable publishing of internal crate

## 0.0.8 (2025-12-11)

### Fixes

#### Prevent planner failure when combining conditional directives and interfaces

Fixed a bug where the query planner failed to handle the combination of conditional directives (`@include`/`@skip`) and the automatic `__typename` injection required for abstract types.

## 0.0.7 (2025-12-08)

### Fixes

- Bump dependencies

## 0.0.6 (2025-11-28)

### Fixes

- make supergraph.{path,key,endpoint} optional (#593)

## 0.0.5 (2025-11-28)

### Fixes

- support `@include` and `@skip` in initial fetch node (#591)
- Fixed an issue where `@skip` and `@include` directives were incorrectly removed from the initial Fetch of the Query Plan.

## 0.0.4 (2025-11-24)

### Fixes

#### Avoid extra `query` prefix for anonymous queries

When there is no variable definitions and no operation name, GraphQL queries can be sent without the `query` prefix. For example, instead of sending:

```diff
- query {
+ {
  user(id: "1") {
    name
  }
}
```

## 0.0.3 (2025-11-06)

### Fixes

#### CommonJS bindings

Adding support for CJS.

## 0.0.2 (2025-11-05)

### Features

#### A node addon containing the query planner of hive router

To use in TypeScript, you would go ahead and do something like:

```ts
import {
  QueryPlanner,
  type QueryPlan,
} from "@graphql-hive/router-query-planner";

const supergraphSdl = "<your sdl from file or in code>";

const qp = new QueryPlanner(supergraphSdl);

const plan: QueryPlan = qp.plan(/* GraphQL */ `
  {
    posts {
      title
      author {
        name
      }
    }
  }
`);

// see QueryPlan types in lib/node-addon/src/query-plan.d.ts
```

which will generate you a [Query Plan used in Apollo Federation](https://www.apollographql.com/docs/graphos/schema-design/federated-schemas/reference/query-plans).

Hive Gateway uses it as an alternative federation query planner in the [`@graphql-hive/router-runtime`](https://github.com/graphql-hive/gateway/blob/main/packages/router-runtime).

To use in with Hive Gateway, you first install the runtime

```sh
npm i @graphql-hive/router-runtime
```

```ts
// gateway.config.ts
import { defineConfig } from "@graphql-hive/gateway";
import { unifiedGraphHandler } from "@graphql-hive/router-runtime";

export const gatewayConfig = defineConfig({
  unifiedGraphHandler,
});
```
