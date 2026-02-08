## 0.5.1 (2026-02-06)

### Features

- Operation Complexity - Limit Aliases (#746)
- Operation Complexity - Limit Aliases (#749)

## 0.5.0 (2026-01-22)

### Breaking Changes

#### Refactor Parse Error Handling in `graphql-tools`

Breaking;
- `ParseError(String)` is now `ParseError(InternalError<'static>)`.
- - So that the internals of the error can be better structured and more informative, such as including line and column information.
- `ParseError`s are no longer prefixed with "query parse error: " in their Display implementation.

## 0.4.2 (2026-01-16)

### Fixes

#### Add `minify_query_document` for optimized query minification

Implements `minify_query_document` to minify parsed GraphQL operations directly, avoiding the need for an intermediate `Display` step. This new approach uses `itoa` and `ryu` for efficient integer and float formatting.

By minifying the query document representation instead of the query string, we achieve performance improvements: query minification time is reduced from 4μs to 500ns, and unnecessary allocations are eliminated.

Includes benchmarks and tests to validate the performance gains and correctness of the new implementation.

## 0.4.1 (2026-01-14)

### Fixes

#### Moves `graphql-tools` to router repository

This change moves the `graphql-tools` package to the Hive Router repository.

## Own GraphQL Parser

This change also introduces our own GraphQL parser (copy of `graphql_parser`), which is now used across all packages in the Hive Router monorepo. This allows us to have better control over parsing and potentially optimize it for our specific use cases.
