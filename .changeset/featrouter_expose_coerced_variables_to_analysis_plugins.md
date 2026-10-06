---
hive-router: minor
---

# Expose coerced variables in `on_graphql_analysis`

`OnGraphqlAnalysisHookPayload` now has two methods for reading the operation's variables after defaults and input coercion:

- `coerced_variable(name)` returns the value of a single variable.
- `coerced_variables()` returns all variables.

The raw values in `graphql_params` only hold what the client sent, so a variable the client left out is missing there even when the operation declares a default. These methods return the value that execution uses.

Closes https://github.com/graphql-hive/router/issues/1650
