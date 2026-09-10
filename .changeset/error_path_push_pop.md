---
hive-router: patch
---

# Error paths in a subgraph response are no longer rebuilt at every step

When a subgraph response contains errors, the router walks the response data and builds a GraphQL
error path for each entity it visits.

It used to build that path by copying. At every level, for every item it visited, it copied the
whole path built so far and added one segment.

The path is now carried through the walk. A segment is added when the walk goes deeper, and
removed when it comes back. Only the leaf needs to own a path, so only the leaf allocates one.

Counting the allocations both versions make over the same data, the saving is around a third for
the shallow two-level paths that are most common, and between a half and three fifths for deeper
ones.

This only runs when a subgraph returns an error. Responses without errors build no path in either
version.
