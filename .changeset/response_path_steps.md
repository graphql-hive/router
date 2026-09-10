---
hive-router: patch
---

# Response paths in a query plan own their steps

A flatten node has a path, and a batched entity fetch has merge paths. Both were stored as a `Vec`
of steps. Each step held a `String` field name and an inline `BTreeSet<String>` of type conditions.

A path is built once when the plan is built, and then only read. It is now stored as a boxed slice
of steps, and each step owns exactly what it holds: a boxed field name, and a boxed type condition
made of boxed names. Code that walks a path now takes a borrowed view of it instead of a reference
to the owner.

A stored path step goes from 32 to 24 bytes, a stored path from 24 to 16, and the merge-path list
on an entity batch alias from 24 to 16.

This does not reduce memory overall. Boxing the type condition adds one allocation for every step
that has one, which roughly cancels out the smaller steps. The change is about how paths are held,
so that the guarantee below can be enforced.

The type names in a condition are sorted, and duplicates are removed, when the condition is built.
This happens whichever way it is built: by the planner, or by deserializing a plan that came back
from a plugin or from `node-addon`. `BTreeSet` did this automatically on `main`, and a plain slice
does not.

Without it, two paths that list the same type names in a different order would compare, hash and
serialize differently. A batched entity fetch would then walk the same path twice instead of
sharing it. Three tests check the sorting and deduplication, so the guarantee still holds with the
new representation.

Query plans on the wire are unchanged, byte for byte.
