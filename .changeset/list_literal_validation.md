---
graphql-tools: patch
hive-router: patch
node-addon: patch
---

# Validate list literals in more positions

Validation now checks the items of a list literal whose type is a non-null list. For example, `query ($v: [Color]! = [PURPLE])` is rejected when `PURPLE` is not a `Color` value. Before, those items were not checked, so an invalid default was only caught when it was used, or was sent to the subgraph unchecked.

A list literal in a position that doesn't expect a list, like `query ($v: Color = [RED])` or `query ($v: Int = [1])`, is now rejected with a `ValuesOfCorrectType` error. Custom scalars still accept any literal, as described by the GraphQL specification.
