---
hive-router: patch
---

# Respect direct-only navigation when checking key fragments

Inline fragment checks now honor `use_only_direct_edges`. When the planner checks whether an entity key can be satisfied using only direct edges, fragment branches can no longer use indirect paths through other subgraphs to appear satisfiable.

For example, consider a fragment requirement:

```graphql
... on Tagged {
  tag {
    id
  }
}
```

Suppose reaching `Tagged` requires an indirect path:

```text
Current node in subgraph A -> entity in subgraph B -> Tagged
```

Previously, the fragment check could use that path even with `use_only_direct_edges: true`, incorrectly accepting the key as locally satisfiable. Now it rejects that candidate unless a direct path exists or the current type already matches `Tagged`. Checks that allow indirect edges retain their existing behavior.
