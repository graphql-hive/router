---
hive-router: patch
---

# Don't ask subgraphs for `__typename` of objects whose type is known

To fetch more fields of an entity from another subgraph, the router used to ask the first subgraph for the entity's `__typename` along with its key, for every object. When the field returns an object type, and not an interface or a union, every object there has that type, so the router now fills in `__typename` itself, and the subgraph doesn't send it.

Subgraph requests and responses get smaller: in a query with nested entities, `__typename` was on every nested entity object. Entity reuse and deduplication treat an entity without `__typename` as the type it's known to have, so they match it with the same entity fetched with one. A child fetch to the same subgraph also merges into its parent when the parent doesn't ask for `__typename`, which saves a fetch in some plans.
