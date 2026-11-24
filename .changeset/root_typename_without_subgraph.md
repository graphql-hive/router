---
hive-router: patch
---

# Root `__typename` is answered without a subgraph request

`{ __typename }` and `... on Mutation { __typename }` used to be sent to a subgraph, even though the router writes the root type's name into the response by itself. The router now leaves root `__typename` out of subgraph requests, so an operation that only asks for it makes no request at all, and root fetches no longer carry it. Subscriptions still send it, since they have to select their one field from a subgraph.

This doesn't go through introspection, so it works when introspection is disabled.
