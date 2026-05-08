---
graphql-tools: patch
---

# Use Mutation and Subscription as default names for root types

`graphql-tools ` assumed the schema definition object provides them, but in case the schema definition object is not present, we use `Mutation` and `Subscription` as default names for root types.
