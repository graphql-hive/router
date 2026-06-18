---
hive-router-plan-executor: patch
hive-router: patch
---

# Fix null propagation in non-null fields

This change fixes the null propagation logic in non-null fields to match the spec.

From the GraphQL spec:

> Since Non-Null response positions cannot be null, execution errors are propagated to be handled by the parent response position. If the parent response position may be null then it resolves to null, otherwise if it is a Non-Null type, the execution error is further propagated to its parent response position.
> If a List type wraps a Non-Null type, and one of the response position elements of that list resolves to null, then the entire list response position must resolve to null. If the List type is also wrapped in a Non-Null, the execution error continues to propagate upwards.
> If every response position from the root of the request to the source of the execution error has a Non-Null type, then the "data" entry in the execution result should be null.

See [Handling Execution Errors](https://spec.graphql.org/September2025/#sec-Handling-Execution-Errors).

Fixes https://github.com/graphql-hive/router/issues/1154

Fixes https://github.com/graphql-hive/router/issues/1110
