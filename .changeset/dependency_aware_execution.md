---
hive-router: minor
---

Run each subgraph fetch as soon as the fetches it reads from are done.

A query plan lists fetches in waves, and every fetch of a wave used to wait for the whole wave before it. In `{ topProducts { upc inStock reviews { author { name } } } }`, the authors only need the reviews, but they also waited for the `inStock` call, so a slow inventory subgraph held them up. Plans now keep which fetches each fetch reads from, and the router starts every fetch the moment those are in. Root mutation fields still run one after another, entity calls included.

A plan that a plugin builds or changes, without that information, runs in its waves like before.
