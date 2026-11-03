---
hive-router: patch
---

# Plan `@requires` that ask for one nested field with other arguments

Planning no longer fails with "Response key 'averagePrice' would hold two different fields" when two `@requires` need the same nested field with other arguments, like `category { averagePrice(currency: "USD") }` and `category { averagePrice(currency: "EUR") }` ([#1829](https://github.com/graphql-hive/router/issues/1829)). The router fetches both, the second under an alias, and sends each to the field that needs it.

Closes #1829
