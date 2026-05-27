---
graphql-tools: patch
hive-router-query-planner: patch
hive-router-plan-executor: patch
hive-console-sdk: patch
hive-router: patch
node-addon: patch
---

# Fix `VariablesInAllowedPosition` rejecting list-typed variables with a non-null default value

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
