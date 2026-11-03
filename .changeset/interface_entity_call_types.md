---
hive-router: patch
---

# Interface entity calls only send the types they ask for

An entity call through an interface key, like `Animal @key(fields: "id")`, sent every implementation, even when everything it asked for was under `... on Dog` and `... on Cat`. Birds went to the subgraph with nothing to resolve. Such calls now only send the types the selection asks for.
