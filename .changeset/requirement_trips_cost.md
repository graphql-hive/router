---
hive-router: patch
node-addon: patch
---

# Plans count the trip a key or `@requires` takes

When a field's `@requires` (or an entity call's key) needs fields from another subgraph, getting them is one more trip, and one more wave the plan waits for. The planner used to count only the size of those fields. So it could pick a subgraph whose `@requires` needed that extra trip, and skip a subgraph that already has the fields, which would have saved a wave.

Now a path pays for those trips. A trip into a subgraph the plan already fetches the object from costs nothing, since its fields can come in that same call. Some plans now have one wave and one request fewer.
