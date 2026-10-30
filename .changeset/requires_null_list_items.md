---
hive-router: patch
---

# Keep `null` list items in `@requires` representations

A list a `@requires` reads, like `similar: [Book]` in `@requires(fields: "similar { id }")`, lost its `null` items on the way to the subgraph. `[null, {id: "2"}, null, {id: "3"}]` arrived as `[{id: "2"}, {id: "3"}]`, so a resolver that counts the items or reads their positions got the wrong answer. Every item now keeps its place, and one with nothing to send is sent as `null`.
