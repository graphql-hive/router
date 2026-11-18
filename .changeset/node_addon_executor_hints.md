---
node-addon: patch
---

# Entity keys, possible types and the response projection

The `QueryPlanner` now has `entityKeys`, each subgraph's resolvable `@key`s by entity type, and `possibleTypes`, the types of each interface and union, the way the router's executor sees them. An executor no longer has to read them from the supergraph itself.

Every plan now also has its `projection`, the operation's selections merged and resolved the way the router shapes the response with them: each field with its alias, nullability, the parent types it applies to and the conditions it's projected under.
